# I02 error and durability contract

This note describes the schema-1 model errors and the storage errors exposed by `cm::profiles`. Errors are safe diagnostics for callers; they do not include credential bytes, endpoint material, URLs, command arguments, environment values, filesystem paths, raw serde messages, or operating-system error text.

## Model validation

`ModelError` is a payload-free enum. Its `Display` output is a fixed description, not the offending record. It reports these categories:

| Category | Variants |
|---|---|
| Schema and identifiers | `InvalidId`, `UnsupportedSchemaVersion`, `DuplicateId`, `KeyMismatch`, `IdReuse`, `IssuedIdsShrank` |
| Graph references | `MissingEntity`, `InvalidReference`, `InvalidOwnership`, `SourceInUse` |
| Values and lifecycle | `InvalidGeneration`, `InvalidDigest`, `InvalidPolicy`, `InvalidLifecycle`, `InvalidCredentialMetadata`, `InvalidVerification`, `InvalidRemovalStage` |
| Immutable transition rules | `NodeMutation`, `ActiveSessionPinsChanged`, `SourceGenerationChangedIncorrectly`, `RevisionChangedIncorrectly` |

The model validator receives typed IDs constrained to ASCII `[A-Za-z0-9_-]` with a length of 1–128 bytes. It does not echo the unchecked value that failed validation.

## Store errors

`StoreError::code()` returns the corresponding stable `StoreErrorCode`; storage callers should branch on that code or the typed variant rather than parse `Display` text.

| Variant / code | Meaning and safe context |
|---|---|
| `InvalidModel` | The proposed graph violates a `ModelError` invariant. |
| `UnsupportedSchema` / `CorruptState` | State has an unsupported schema version, or cannot be strictly decoded and validated. These results are distinct from I/O errors. |
| `Conflict { expected, current }` | Graph revision compare-and-swap failed. Only revision numbers are included. |
| `SourceGenerationConflict { expected, current }` | Source generation compare-and-swap failed. Only generation numbers are included. |
| `Busy` | Bounded advisory-lock wait expired. This is distinct from lock I/O failure. |
| `UnsafeFilesystem` / `AlreadyInitialized` | A checked directory, file, identity, mode, owner, link count, or initialization target is unsafe or already present. |
| `CredentialMismatch` / `CredentialOccupied { credential_ref }` / `MissingCredential { credential_ref }` | Credential metadata does not match verified bytes, an immutable blob name is occupied, or an expected blob is missing. Only the typed credential ID can be present. |
| `PendingRemoval { source_id }` / `NotFound` | The source cannot be used during staged removal, or the requested entity/plan does not exist. |
| `SourceInUse { source_id, references }` | The source still has live typed references. The diagnostic lists source, profile, or active-session IDs. |
| `ImmutableNode { node_id, references }` | A node ID was changed after publication. The diagnostic lists the node ID and referencing profile/session IDs. |
| `Io { operation, kind }` | An operation code and `std::io::ErrorKind`; no raw OS message, path, or filename is exposed. |
| `DurabilityIndeterminate` | State rename succeeded, but syncing the containing store directory failed. The caller must reopen and inspect the published revision before retrying. |

`CredentialMaterial` is not serializable. Its `Debug` output is redacted; trusted consumers can access bytes only through the explicit `as_bytes()` method.

## Publication and recovery

Before state rename, a failure leaves the old graph published. Newly created immutable blobs may remain as private, unpublished orphans; they are never overwritten or removed by a name-prefix sweep. A later attempt with an occupied credential ID fails with `CredentialOccupied`, so the caller must choose a fresh ID or arrange an explicit recovery.

After state rename, a directory-sync failure is reported as `DurabilityIndeterminate`. The store does not claim rollback. Reopening determines whether the new complete snapshot is visible; a retry uses the observed revision.

Source removal first publishes one `BlobPending` snapshot containing the exact credential metadata set. It then verifies the complete blob set before unlinking, removes only verified files, syncs the credentials directory, and publishes the completion snapshot. If removal stops after some unlinks, the durable pending plan remains retryable; missing blobs are tolerated only when named by that pending plan. A corrupt or foreign planned blob stops removal before the first unlink. If the final state rename succeeds but its directory sync fails, report `DurabilityIndeterminate` and reopen to determine whether the complete or pending revision is visible.

Lock serialization and revision CAS apply to cooperating Store clients using the permanent advisory lock. The filesystem checks fail closed for detected replacements, but they do not promise an atomic compare-and-unlink against an uncoordinated same-UID writer between the last pathname check and `unlinkat`.

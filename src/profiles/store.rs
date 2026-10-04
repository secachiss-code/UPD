//! Private, revisioned profile graph storage.
//!
//! Credential payloads live in immutable private blob files. This layer has no
//! networking, worker, service-manager, or legacy-import responsibilities.

use super::model::{
    CredentialMetadata, EntityIdKind, GraphSnapshot, HostMode, Id, ModelError, Node, RemovalStage,
    SessionLifecycle, Source, TunnelLifecycle, Verification, VerificationAxis, VerificationValue,
    source_removal_candidates,
};
use libc::{self, AT_SYMLINK_NOFOLLOW, O_CLOEXEC, O_DIRECTORY, O_NOFOLLOW, O_NONBLOCK};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::CString;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const STORE_DIRECTORY_MODE: u32 = 0o700;
const PRIVATE_FILE_MODE: u32 = 0o600;
const MAX_STATE_BYTES: usize = 32 * 1024 * 1024;
const MAX_CREDENTIAL_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_LOCK_TIMEOUT: Duration = Duration::from_secs(30);
const LOCK_FILE: &str = ".lock";
const STATE_FILE: &str = "state.json";
const CREDENTIAL_DIRECTORY: &str = "credentials";
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

// Per-instance failure seams exist only in the test build; production has no
// environment switches or public injection API.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TestFault {
    BlobSync,
    StateTempSync,
    StateRename,
    StateDirectorySync,
    AfterBlobUnlink,
}

macro_rules! test_fault {
    ($store:expr, $point:ident) => {
        #[cfg(test)]
        $store.inject_test_fault(TestFault::$point)?;
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoreIoOperation {
    Open,
    Read,
    Write,
    Create,
    Rename,
    Unlink,
    Sync,
    Lock,
    Stat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoreErrorCode {
    InvalidModel,
    UnsupportedSchema,
    CorruptState,
    Conflict,
    SourceGenerationConflict,
    Busy,
    UnsafeFilesystem,
    CredentialMismatch,
    CredentialOccupied,
    MissingCredential,
    PendingRemoval,
    NotFound,
    SourceInUse,
    ImmutableNode,
    Io,
    DurabilityIndeterminate,
    AlreadyInitialized,
}

#[derive(Debug)]
pub enum StoreError {
    InvalidModel(ModelError),
    UnsupportedSchema,
    CorruptState,
    Conflict {
        expected: u64,
        current: u64,
    },
    SourceGenerationConflict {
        expected: u64,
        current: u64,
    },
    Busy,
    UnsafeFilesystem,
    CredentialMismatch,
    CredentialOccupied {
        credential_ref: Id,
    },
    MissingCredential {
        credential_ref: Id,
    },
    PendingRemoval {
        source_id: Id,
    },
    NotFound,
    SourceInUse {
        source_id: Id,
        references: Vec<Id>,
    },
    ImmutableNode {
        node_id: Id,
        references: Vec<Id>,
    },
    Io {
        operation: StoreIoOperation,
        kind: io::ErrorKind,
    },
    DurabilityIndeterminate,
    AlreadyInitialized,
}

impl StoreError {
    pub fn code(&self) -> StoreErrorCode {
        match self {
            Self::InvalidModel(_) => StoreErrorCode::InvalidModel,
            Self::UnsupportedSchema => StoreErrorCode::UnsupportedSchema,
            Self::CorruptState => StoreErrorCode::CorruptState,
            Self::Conflict { .. } => StoreErrorCode::Conflict,
            Self::SourceGenerationConflict { .. } => StoreErrorCode::SourceGenerationConflict,
            Self::Busy => StoreErrorCode::Busy,
            Self::UnsafeFilesystem => StoreErrorCode::UnsafeFilesystem,
            Self::CredentialMismatch => StoreErrorCode::CredentialMismatch,
            Self::CredentialOccupied { .. } => StoreErrorCode::CredentialOccupied,
            Self::MissingCredential { .. } => StoreErrorCode::MissingCredential,
            Self::PendingRemoval { .. } => StoreErrorCode::PendingRemoval,
            Self::NotFound => StoreErrorCode::NotFound,
            Self::SourceInUse { .. } => StoreErrorCode::SourceInUse,
            Self::ImmutableNode { .. } => StoreErrorCode::ImmutableNode,
            Self::Io { .. } => StoreErrorCode::Io,
            Self::DurabilityIndeterminate => StoreErrorCode::DurabilityIndeterminate,
            Self::AlreadyInitialized => StoreErrorCode::AlreadyInitialized,
        }
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidModel(error) => write!(f, "invalid profile graph: {error}"),
            Self::UnsupportedSchema => f.write_str("unsupported profile schema"),
            Self::CorruptState => f.write_str("profile state is corrupt"),
            Self::Conflict { expected, current } => {
                write!(f, "profile revision conflict ({expected}, {current})")
            }
            Self::SourceGenerationConflict { expected, current } => {
                write!(f, "source generation conflict ({expected}, {current})")
            }
            Self::Busy => f.write_str("profile store is busy"),
            Self::UnsafeFilesystem => f.write_str("unsafe profile store filesystem entry"),
            Self::CredentialMismatch => f.write_str("credential metadata does not match material"),
            Self::CredentialOccupied { credential_ref } => write!(
                f,
                "credential reference is already occupied: {credential_ref}"
            ),
            Self::MissingCredential { credential_ref } => {
                write!(f, "credential is missing: {credential_ref}")
            }
            Self::PendingRemoval { source_id } => {
                write!(f, "source has a pending removal: {source_id}")
            }
            Self::NotFound => f.write_str("profile entity was not found"),
            Self::SourceInUse {
                source_id,
                references,
            } => write!(
                f,
                "source {source_id} is still referenced by {references:?}"
            ),
            Self::ImmutableNode {
                node_id,
                references,
            } => write!(
                f,
                "node {node_id} is immutable and referenced by {references:?}"
            ),
            Self::Io { operation, kind } => {
                write!(f, "profile store {operation:?} failed ({kind:?})")
            }
            Self::DurabilityIndeterminate => {
                f.write_str("profile state was renamed but directory durability is indeterminate")
            }
            Self::AlreadyInitialized => f.write_str("profile store is already initialized"),
        }
    }
}

impl std::error::Error for StoreError {}

/// Secret input buffer. Formatting is always redacted and it is never serializable.
pub struct CredentialMaterial {
    credential_ref: Id,
    bytes: Vec<u8>,
}

impl CredentialMaterial {
    pub fn new(credential_ref: Id, bytes: Vec<u8>) -> Result<Self, StoreError> {
        if bytes.len() > MAX_CREDENTIAL_BYTES {
            return Err(StoreError::CredentialMismatch);
        }
        Ok(Self {
            credential_ref,
            bytes,
        })
    }

    pub fn credential_ref(&self) -> &Id {
        &self.credential_ref
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Explicitly expose bytes to trusted credential consumers; never used by status serialization.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl fmt::Debug for CredentialMaterial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CredentialMaterial")
            .field("credential_ref", &self.credential_ref)
            .field("bytes", &"[REDACTED]")
            .field("len", &self.bytes.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct FileStamp {
    size: u64,
    mtime_seconds: i64,
    mtime_nanoseconds: i64,
    ctime_seconds: i64,
    ctime_nanoseconds: i64,
    mode: u32,
    uid: u32,
    gid: u32,
    links: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StoreIdentity {
    root: FileIdentity,
    credentials: FileIdentity,
    lock: FileIdentity,
}

struct StateRead {
    graph: GraphSnapshot,
    file: FileIdentity,
    digest: [u8; 32],
}

pub struct Store {
    root_path: PathBuf,
    root_fd: File,
    credentials_fd: File,
    lock_fd: File,
    uid: u32,
    identity: StoreIdentity,
    lock_timeout: Duration,
    #[cfg(test)]
    test_fault: std::sync::Mutex<Option<TestFault>>,
}

impl Store {
    /// Create a new explicitly named store root. The path itself must not exist.
    pub fn initialize(root: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = validate_root_argument(root.as_ref())?;
        let name = path
            .file_name()
            .ok_or(StoreError::UnsafeFilesystem)?
            .to_str()
            .ok_or(StoreError::UnsafeFilesystem)?;
        let parent_path = path.parent().ok_or(StoreError::UnsafeFilesystem)?;
        let parent_fd = open_absolute_directory(parent_path)?;
        validate_ancestor_chain(&path)?;
        let parent_metadata = parent_fd
            .metadata()
            .map_err(|e| io_error(StoreIoOperation::Stat, e))?;
        verify_safe_ancestor(&parent_metadata, parent_path)?;
        let name = cstring(name)?;
        let uid = unsafe { libc::geteuid() };
        if unsafe { libc::mkdirat(parent_fd.as_raw_fd(), name.as_ptr(), STORE_DIRECTORY_MODE) } != 0
        {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::AlreadyExists {
                return Err(StoreError::AlreadyInitialized);
            }
            return Err(io_error(StoreIoOperation::Create, error));
        }
        parent_fd
            .sync_all()
            .map_err(|e| io_error(StoreIoOperation::Sync, e))?;
        let root_fd = openat_file(
            &parent_fd,
            name.to_str().map_err(|_| StoreError::UnsafeFilesystem)?,
            libc::O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC,
            0,
        )
        .map_err(|e| io_error(StoreIoOperation::Open, e))?;
        fchmod_exact(&root_fd, STORE_DIRECTORY_MODE)?;
        verify_directory(&root_fd, uid, STORE_DIRECTORY_MODE)?;

        if unsafe {
            libc::mkdirat(
                root_fd.as_raw_fd(),
                cstring(CREDENTIAL_DIRECTORY)?.as_ptr(),
                STORE_DIRECTORY_MODE,
            )
        } != 0
        {
            return Err(io_error(
                StoreIoOperation::Create,
                io::Error::last_os_error(),
            ));
        }
        root_fd
            .sync_all()
            .map_err(|e| io_error(StoreIoOperation::Sync, e))?;
        let credentials_fd = openat_file(
            &root_fd,
            CREDENTIAL_DIRECTORY,
            libc::O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC,
            0,
        )
        .map_err(|e| io_error(StoreIoOperation::Open, e))?;
        fchmod_exact(&credentials_fd, STORE_DIRECTORY_MODE)?;
        verify_directory(&credentials_fd, uid, STORE_DIRECTORY_MODE)?;
        credentials_fd
            .sync_all()
            .map_err(|e| io_error(StoreIoOperation::Sync, e))?;

        let lock_fd = openat_file(
            &root_fd,
            LOCK_FILE,
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC,
            PRIVATE_FILE_MODE,
        )
        .map_err(|e| io_error(StoreIoOperation::Create, e))?;
        fchmod_exact(&lock_fd, PRIVATE_FILE_MODE)?;
        verify_private_file(&lock_fd, uid, PRIVATE_FILE_MODE)?;
        lock_fd
            .sync_all()
            .map_err(|e| io_error(StoreIoOperation::Sync, e))?;
        root_fd
            .sync_all()
            .map_err(|e| io_error(StoreIoOperation::Sync, e))?;

        let store = Self::from_open_fds(
            path,
            root_fd,
            credentials_fd,
            lock_fd,
            uid,
            DEFAULT_LOCK_TIMEOUT,
        )?;
        let _guard = store.lock(true)?;
        let initial = GraphSnapshot::new();
        initial.validate().map_err(StoreError::InvalidModel)?;
        let encoded = encode_graph(&initial)?;
        store.publish_locked(None, &encoded)?;
        Ok(store)
    }

    /// Open and validate an existing explicitly named private store.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = validate_root_argument(root.as_ref())?;
        validate_ancestor_chain(&path)?;
        let root_fd = open_absolute_directory(&path)?;
        let uid = unsafe { libc::geteuid() };
        verify_directory(&root_fd, uid, STORE_DIRECTORY_MODE)?;
        let credentials_fd = openat_file(
            &root_fd,
            CREDENTIAL_DIRECTORY,
            libc::O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC,
            0,
        )
        .map_err(|e| classify_open_error(e, StoreIoOperation::Open))?;
        verify_directory(&credentials_fd, uid, STORE_DIRECTORY_MODE)?;
        let lock_fd = openat_file(
            &root_fd,
            LOCK_FILE,
            libc::O_RDWR | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC,
            0,
        )
        .map_err(|e| classify_open_error(e, StoreIoOperation::Open))?;
        verify_private_file(&lock_fd, uid, PRIVATE_FILE_MODE)?;
        let store = Self::from_open_fds(
            path,
            root_fd,
            credentials_fd,
            lock_fd,
            uid,
            DEFAULT_LOCK_TIMEOUT,
        )?;
        let _guard = store.lock(false)?;
        store.read_state_locked()?;
        Ok(store)
    }

    /// Adjust the bounded flock wait, capped at 30 seconds.
    pub fn with_lock_timeout(mut self, timeout: Duration) -> Self {
        self.lock_timeout = timeout.min(MAX_LOCK_TIMEOUT);
        self
    }

    /// Read one whole validated snapshot under a shared lock.
    pub fn read_snapshot(&self) -> Result<GraphSnapshot, StoreError> {
        let _guard = self.lock(false)?;
        let state = self.read_state_locked()?;
        Ok(state.graph)
    }

    /// Commit a complete graph candidate and any newly referenced credential material.
    pub fn commit(
        &self,
        expected_revision: u64,
        mut candidate: GraphSnapshot,
        materials: Vec<CredentialMaterial>,
    ) -> Result<GraphSnapshot, StoreError> {
        let _guard = self.lock(true)?;
        let current = self.read_state_locked()?;
        self.check_revision(expected_revision, current.graph.revision)?;
        if candidate.staged_removals != current.graph.staged_removals
            || !current
                .graph
                .credentials
                .iter()
                .all(|(id, metadata)| candidate.credentials.get(id) == Some(metadata))
        {
            return Err(StoreError::InvalidModel(ModelError::InvalidRemovalStage));
        }
        candidate.revision = current
            .graph
            .revision
            .checked_add(1)
            .ok_or(StoreError::CorruptState)?;
        self.publish_candidate_locked(&current, &candidate, materials, false)
    }

    /// Read a credential only through this explicit private API.
    pub fn read_credential(&self, credential_ref: &Id) -> Result<CredentialMaterial, StoreError> {
        let _guard = self.lock(false)?;
        let current = self.read_state_locked()?;
        let metadata = current
            .graph
            .credentials
            .get(credential_ref)
            .ok_or_else(|| StoreError::MissingCredential {
                credential_ref: credential_ref.clone(),
            })?;
        if let Some(plan) = current.graph.staged_removals.values().find(|plan| {
            plan.stage != RemovalStage::Complete
                && plan
                    .credential_metadata
                    .iter()
                    .any(|item| &item.credential_ref == credential_ref)
                && !plan.removed_credential_refs.contains(credential_ref)
        }) {
            return Err(StoreError::PendingRemoval {
                source_id: plan.source_id.clone(),
            });
        }
        let bytes = self.read_blob_verified(metadata)?;
        Ok(CredentialMaterial {
            credential_ref: credential_ref.clone(),
            bytes,
        })
    }

    /// Begin source removal in one durable snapshot. No blob is unlinked by this call.
    pub fn begin_removal(
        &self,
        expected_revision: u64,
        plan_id: Id,
        source_id: Id,
    ) -> Result<GraphSnapshot, StoreError> {
        let _guard = self.lock(true)?;
        self.begin_removal_locked(expected_revision, plan_id, source_id)
    }

    /// Finish a durable pending removal, validating all planned blobs before the first unlink.
    pub fn finish_removal(&self, plan_id: &Id) -> Result<GraphSnapshot, StoreError> {
        let _guard = self.lock(true)?;
        self.finish_removal_locked(plan_id)
    }

    /// Complete source removal while retaining one exclusive store lock for both durable stages.
    pub fn remove_source(
        &self,
        expected_revision: u64,
        plan_id: Id,
        source_id: Id,
    ) -> Result<GraphSnapshot, StoreError> {
        let _guard = self.lock(true)?;
        self.begin_removal_locked(expected_revision, plan_id.clone(), source_id)?;
        self.finish_removal_locked(&plan_id)
    }

    /// Replace one Source generation and its Nodes atomically under a single exclusive lock.
    pub fn update_source(
        &self,
        expected_revision: u64,
        expected_source_generation: u64,
        source: Source,
        nodes: Vec<Node>,
        materials: Vec<CredentialMaterial>,
        evidence_at_unix_ms: i64,
    ) -> Result<GraphSnapshot, StoreError> {
        let _guard = self.lock(true)?;
        let current = self.read_state_locked()?;
        self.check_revision(expected_revision, current.graph.revision)?;
        let old_source = current
            .graph
            .sources
            .get(&source.id)
            .ok_or(StoreError::NotFound)?;
        if old_source.generation != expected_source_generation {
            return Err(StoreError::SourceGenerationConflict {
                expected: expected_source_generation,
                current: old_source.generation,
            });
        }
        if current
            .graph
            .staged_removals
            .values()
            .any(|plan| plan.source_id == source.id && plan.stage != RemovalStage::Complete)
        {
            return Err(StoreError::PendingRemoval {
                source_id: source.id,
            });
        }
        let mut candidate = current.graph.clone();
        let old_digest_set: BTreeSet<String> = old_source
            .current_node_ids
            .iter()
            .filter_map(|node_id| current.graph.nodes.get(node_id))
            .map(|node| node.definition_digest_sha256.clone())
            .collect();
        let node_input_count = nodes.len();
        let new_nodes_by_id: BTreeMap<Id, Node> = nodes
            .into_iter()
            .map(|node| (node.id.clone(), node))
            .collect();
        if new_nodes_by_id.len() != node_input_count
            || new_nodes_by_id.len() != source.current_node_ids.len()
            || source
                .current_node_ids
                .iter()
                .any(|id| !new_nodes_by_id.contains_key(id))
        {
            return Err(StoreError::InvalidModel(ModelError::InvalidReference));
        }
        for (id, node) in new_nodes_by_id {
            if id != node.id
                || node.source_id != source.id
                || node.source_generation != source.generation
            {
                return Err(StoreError::InvalidModel(ModelError::InvalidReference));
            }
            if !candidate.nodes.contains_key(&id) {
                reserve_id(&mut candidate, id.clone(), EntityIdKind::Node)?;
            }
            candidate.nodes.insert(id, node);
        }
        if let Some(metadata) = &source.credential {
            ensure_material_metadata(
                &current.graph,
                &mut candidate,
                metadata,
                &materials,
                &source.id,
            )?;
        }
        for node_id in &source.current_node_ids {
            let credential_refs = candidate
                .nodes
                .get(node_id)
                .ok_or(StoreError::InvalidModel(ModelError::MissingEntity))?
                .credential_refs
                .clone();
            for credential_ref in &credential_refs {
                if !candidate.credentials.contains_key(credential_ref) {
                    let material = materials
                        .iter()
                        .find(|material| &material.credential_ref == credential_ref)
                        .ok_or_else(|| StoreError::MissingCredential {
                            credential_ref: credential_ref.clone(),
                        })?;
                    let metadata =
                        make_metadata(credential_ref.clone(), source.id.clone(), &material.bytes);
                    reserve_id(
                        &mut candidate,
                        credential_ref.clone(),
                        EntityIdKind::CredentialRef,
                    )?;
                    candidate
                        .credentials
                        .insert(credential_ref.clone(), metadata);
                }
            }
        }
        candidate.sources.insert(source.id.clone(), source.clone());

        let new_digests: BTreeSet<String> = source
            .current_node_ids
            .iter()
            .filter_map(|node_id| candidate.nodes.get(node_id))
            .map(|node| node.definition_digest_sha256.clone())
            .collect();
        for session in current.graph.sessions.values().filter(|session| {
            session.lifecycle == SessionLifecycle::Active && session.source_id == source.id
        }) {
            let lost = current
                .graph
                .nodes
                .get(&session.node_id)
                .is_some_and(|node| {
                    old_digest_set.contains(&node.definition_digest_sha256)
                        && !new_digests.contains(&node.definition_digest_sha256)
                });
            if !lost {
                continue;
            }
            let existing = candidate.verifications.values_mut().find(|verification| {
                verification.session_id == session.id && verification.axis == VerificationAxis::Net
            });
            if let Some(verification) = existing {
                verification.value = VerificationValue::Blocked;
                verification.evidence_at_unix_ms = evidence_at_unix_ms;
            } else {
                let id = fresh_id(&candidate, "net")?;
                reserve_id(&mut candidate, id.clone(), EntityIdKind::Verification)?;
                candidate.verifications.insert(
                    id.clone(),
                    Verification {
                        schema_version: super::model::SCHEMA_VERSION,
                        id,
                        session_id: session.id.clone(),
                        tunnel_instance_id: session.tunnel_instance_id.clone(),
                        tunnel_generation: session.tunnel_generation,
                        axis: VerificationAxis::Net,
                        value: VerificationValue::Blocked,
                        evidence_at_unix_ms,
                    },
                );
            }
        }
        candidate.revision = current
            .graph
            .revision
            .checked_add(1)
            .ok_or(StoreError::CorruptState)?;
        self.publish_candidate_locked(&current, &candidate, materials, false)
    }

    /// Stop only the modeled host policy and its own tunnel instance.
    pub fn stop_host(&self, expected_revision: u64) -> Result<GraphSnapshot, StoreError> {
        let _guard = self.lock(true)?;
        let current = self.read_state_locked()?;
        self.check_revision(expected_revision, current.graph.revision)?;
        let Some(host) = current.graph.host_policy.as_ref() else {
            return Ok(current.graph);
        };
        if host.mode == HostMode::Off {
            return Ok(current.graph);
        }
        let mut candidate = current.graph.clone();
        let old_tunnel_id = host
            .tunnel_instance_id
            .clone()
            .ok_or(StoreError::CorruptState)?;
        let host = candidate
            .host_policy
            .as_mut()
            .ok_or(StoreError::CorruptState)?;
        host.mode = HostMode::Off;
        host.connection_profile_id = None;
        host.tunnel_instance_id = None;
        let tunnel = candidate
            .tunnel_instances
            .get_mut(&old_tunnel_id)
            .ok_or(StoreError::CorruptState)?;
        tunnel.lifecycle = TunnelLifecycle::Stopped;
        candidate.revision = current
            .graph
            .revision
            .checked_add(1)
            .ok_or(StoreError::CorruptState)?;
        self.publish_candidate_locked(&current, &candidate, Vec::new(), false)
    }

    fn from_open_fds(
        root_path: PathBuf,
        root_fd: File,
        credentials_fd: File,
        lock_fd: File,
        uid: u32,
        lock_timeout: Duration,
    ) -> Result<Self, StoreError> {
        verify_directory(&root_fd, uid, STORE_DIRECTORY_MODE)?;
        verify_directory(&credentials_fd, uid, STORE_DIRECTORY_MODE)?;
        verify_private_file(&lock_fd, uid, PRIVATE_FILE_MODE)?;
        let store = Self {
            root_path,
            identity: StoreIdentity {
                root: file_identity(&root_fd)?,
                credentials: file_identity(&credentials_fd)?,
                lock: file_identity(&lock_fd)?,
            },
            root_fd,
            credentials_fd,
            lock_fd,
            uid,
            lock_timeout,
            #[cfg(test)]
            test_fault: std::sync::Mutex::new(None),
        };
        store.verify_bindings()?;
        Ok(store)
    }

    #[cfg(test)]
    fn inject_test_fault(&self, point: TestFault) -> Result<(), StoreError> {
        let mut pending = self.test_fault.lock().unwrap();
        if *pending != Some(point) {
            return Ok(());
        }
        *pending = None;
        if point == TestFault::StateDirectorySync {
            return Err(StoreError::DurabilityIndeterminate);
        }
        let operation = match point {
            TestFault::BlobSync | TestFault::StateTempSync => StoreIoOperation::Sync,
            TestFault::StateRename => StoreIoOperation::Rename,
            TestFault::AfterBlobUnlink => StoreIoOperation::Unlink,
            TestFault::StateDirectorySync => unreachable!(),
        };
        Err(StoreError::Io {
            operation,
            kind: io::ErrorKind::Other,
        })
    }

    fn lock(&self, exclusive: bool) -> Result<StoreLockGuard, StoreError> {
        self.verify_bindings()?;
        let lock = openat_file(
            &self.root_fd,
            LOCK_FILE,
            libc::O_RDWR | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC,
            0,
        )
        .map_err(|e| classify_open_error(e, StoreIoOperation::Lock))?;
        verify_private_file(&lock, self.uid, PRIVATE_FILE_MODE)?;
        if file_identity(&lock)? != self.identity.lock {
            return Err(StoreError::UnsafeFilesystem);
        }
        let operation = if exclusive {
            libc::LOCK_EX
        } else {
            libc::LOCK_SH
        };
        let start = Instant::now();
        loop {
            let result = unsafe { libc::flock(lock.as_raw_fd(), operation | libc::LOCK_NB) };
            if result == 0 {
                self.verify_bindings()?;
                return Ok(StoreLockGuard { file: lock });
            }
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                if start.elapsed() >= self.lock_timeout {
                    return Err(StoreError::Busy);
                }
                continue;
            }
            if error.kind() != io::ErrorKind::WouldBlock {
                return Err(io_error(StoreIoOperation::Lock, error));
            }
            if start.elapsed() >= self.lock_timeout {
                return Err(StoreError::Busy);
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn verify_bindings(&self) -> Result<(), StoreError> {
        validate_ancestor_chain(&self.root_path)?;
        let path_root = open_absolute_directory(&self.root_path)?;
        verify_directory(&self.root_fd, self.uid, STORE_DIRECTORY_MODE)?;
        verify_directory(&self.credentials_fd, self.uid, STORE_DIRECTORY_MODE)?;
        verify_private_file(&self.lock_fd, self.uid, PRIVATE_FILE_MODE)?;
        if file_identity(&path_root)? != self.identity.root
            || file_identity(&self.root_fd)? != self.identity.root
            || file_identity(&self.credentials_fd)? != self.identity.credentials
            || file_identity(&self.lock_fd)? != self.identity.lock
            || at_identity(&self.root_fd, CREDENTIAL_DIRECTORY, true)? != self.identity.credentials
            || at_identity(&self.root_fd, LOCK_FILE, false)? != self.identity.lock
        {
            return Err(StoreError::UnsafeFilesystem);
        }
        Ok(())
    }

    fn read_state_locked(&self) -> Result<StateRead, StoreError> {
        self.verify_bindings()?;
        let file = openat_file(
            &self.root_fd,
            STATE_FILE,
            libc::O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC,
            0,
        )
        .map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                StoreError::CorruptState
            } else {
                classify_open_error(e, StoreIoOperation::Read)
            }
        })?;
        verify_private_file(&file, self.uid, PRIVATE_FILE_MODE)?;
        let identity = file_identity(&file)?;
        let bytes = read_bounded(&file, MAX_STATE_BYTES, StoreIoOperation::Read)?;
        let digest = sha256(&bytes);
        let graph = decode_graph(&bytes)?;
        graph.validate().map_err(map_model_error)?;
        self.verify_credentials_available(&graph)?;
        Ok(StateRead {
            graph,
            file: identity,
            digest,
        })
    }

    fn verify_credentials_available(&self, graph: &GraphSnapshot) -> Result<(), StoreError> {
        for metadata in graph.credentials.values() {
            match self.read_blob_verified(metadata) {
                Ok(_bytes) => {}
                Err(StoreError::NotFound)
                    if graph.staged_removals.values().any(|plan| {
                        plan.stage == RemovalStage::BlobPending
                            && plan
                                .credential_metadata
                                .iter()
                                .any(|item| item.credential_ref == metadata.credential_ref)
                    }) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn read_blob_verified(&self, metadata: &CredentialMetadata) -> Result<Vec<u8>, StoreError> {
        self.read_blob_verified_with_identity(metadata)
            .map(|(bytes, _, _, _)| bytes)
    }

    fn read_blob_verified_with_identity(
        &self,
        metadata: &CredentialMetadata,
    ) -> Result<(Vec<u8>, FileIdentity, FileStamp, File), StoreError> {
        let file = self.open_blob(metadata)?;
        verify_private_file(&file, self.uid, PRIVATE_FILE_MODE)?;
        let info = file
            .metadata()
            .map_err(|e| io_error(StoreIoOperation::Stat, e))?;
        if info.len() != metadata.size_bytes || info.len() > MAX_CREDENTIAL_BYTES as u64 {
            return Err(StoreError::CredentialMismatch);
        }
        let bytes = read_bounded(&file, MAX_CREDENTIAL_BYTES, StoreIoOperation::Read)?;
        if bytes.len() as u64 != metadata.size_bytes || hex_sha256(&bytes) != metadata.digest_sha256
        {
            return Err(StoreError::CredentialMismatch);
        }
        let after = file
            .metadata()
            .map_err(|e| io_error(StoreIoOperation::Stat, e))?;
        if file_stamp(&info) != file_stamp(&after) {
            return Err(StoreError::CredentialMismatch);
        }
        Ok((bytes, file_identity(&file)?, file_stamp(&after), file))
    }

    fn open_blob(&self, metadata: &CredentialMetadata) -> Result<File, StoreError> {
        let name = blob_name(&metadata.credential_ref);
        openat_file(
            &self.credentials_fd,
            &name,
            libc::O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC,
            0,
        )
        .map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => StoreError::NotFound,
            io::ErrorKind::InvalidInput | io::ErrorKind::PermissionDenied => {
                StoreError::UnsafeFilesystem
            }
            _ => classify_open_error(error, StoreIoOperation::Read),
        })
    }

    fn check_revision(&self, expected: u64, current: u64) -> Result<(), StoreError> {
        if expected == current {
            Ok(())
        } else {
            Err(StoreError::Conflict { expected, current })
        }
    }

    fn publish_candidate_locked(
        &self,
        current: &StateRead,
        candidate: &GraphSnapshot,
        materials: Vec<CredentialMaterial>,
        allow_removal: bool,
    ) -> Result<GraphSnapshot, StoreError> {
        if !allow_removal
            && (candidate.staged_removals != current.graph.staged_removals
                || !current
                    .graph
                    .credentials
                    .iter()
                    .all(|(id, item)| candidate.credentials.get(id) == Some(item)))
        {
            return Err(StoreError::InvalidModel(ModelError::InvalidRemovalStage));
        }
        diagnose_transition(&current.graph, candidate)?;
        GraphSnapshot::validate_transition(&current.graph, candidate).map_err(map_model_error)?;
        let materials = index_materials(materials)?;
        let new_credentials: Vec<&CredentialMetadata> = candidate
            .credentials
            .iter()
            .filter(|(id, _)| !current.graph.credentials.contains_key(*id))
            .map(|(_, metadata)| metadata)
            .collect();
        if materials.len() != new_credentials.len() {
            return Err(StoreError::CredentialMismatch);
        }
        for metadata in &new_credentials {
            let material = materials.get(&metadata.credential_ref).ok_or_else(|| {
                StoreError::MissingCredential {
                    credential_ref: metadata.credential_ref.clone(),
                }
            })?;
            check_material(metadata, material)?;
        }
        if materials.keys().any(|id| {
            !new_credentials
                .iter()
                .any(|item| &item.credential_ref == id)
        }) {
            return Err(StoreError::CredentialMismatch);
        }

        candidate.validate().map_err(map_model_error)?;
        // Bound and serialize the complete candidate before creating any immutable blob.
        let encoded = encode_graph(candidate)?;
        for metadata in &new_credentials {
            let material = materials
                .get(&metadata.credential_ref)
                .ok_or(StoreError::CredentialMismatch)?;
            self.create_blob(metadata, material)?;
        }
        if !new_credentials.is_empty() {
            self.credentials_fd
                .sync_all()
                .map_err(|e| io_error(StoreIoOperation::Sync, e))?;
        }
        self.publish_locked(Some(current), &encoded)?;
        Ok(candidate.clone())
    }

    fn create_blob(
        &self,
        metadata: &CredentialMetadata,
        material: &CredentialMaterial,
    ) -> Result<(), StoreError> {
        self.verify_bindings()?;
        let name = blob_name(&metadata.credential_ref);
        let file = match openat_file(
            &self.credentials_fd,
            &name,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC,
            PRIVATE_FILE_MODE,
        ) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                return Err(StoreError::CredentialOccupied {
                    credential_ref: metadata.credential_ref.clone(),
                });
            }
            Err(error) => return Err(io_error(StoreIoOperation::Create, error)),
        };
        fchmod_exact(&file, PRIVATE_FILE_MODE)?;
        verify_private_file(&file, self.uid, PRIVATE_FILE_MODE)?;
        // A partial file is intentionally retained as a private orphan on failure.
        let mut writer = &file;
        writer
            .write_all(&material.bytes)
            .map_err(|e| io_error(StoreIoOperation::Write, e))?;
        test_fault!(self, BlobSync);
        file.sync_all()
            .map_err(|e| io_error(StoreIoOperation::Sync, e))?;
        Ok(())
    }

    fn publish_locked(
        &self,
        old_state: Option<&StateRead>,
        bytes: &[u8],
    ) -> Result<(), StoreError> {
        self.verify_bindings()?;
        if bytes.len() > MAX_STATE_BYTES {
            return Err(StoreError::CorruptState);
        }
        if let Some(old) = old_state {
            let now = self.read_state_file_only()?;
            if now.file != old.file || now.digest != old.digest {
                return Err(StoreError::UnsafeFilesystem);
            }
        } else if at_identity_optional(&self.root_fd, STATE_FILE)?.is_some() {
            return Err(StoreError::AlreadyInitialized);
        }

        let (name, file) = self.create_state_temp()?;
        let temp_identity = file_identity(&file)?;
        let write_result = (|| {
            verify_private_file(&file, self.uid, PRIVATE_FILE_MODE)?;
            let mut writer = &file;
            writer
                .write_all(bytes)
                .map_err(|e| io_error(StoreIoOperation::Write, e))?;
            test_fault!(self, StateTempSync);
            file.sync_all()
                .map_err(|e| io_error(StoreIoOperation::Sync, e))?;
            self.verify_bindings()?;
            if let Some(old) = old_state {
                let now = self.read_state_file_only()?;
                if now.file != old.file || now.digest != old.digest {
                    return Err(StoreError::UnsafeFilesystem);
                }
            }
            if at_identity(&self.root_fd, &name, false)? != temp_identity {
                return Err(StoreError::UnsafeFilesystem);
            }
            let c_name = cstring(&name)?;
            let state_name = cstring(STATE_FILE)?;
            test_fault!(self, StateRename);
            let renamed = if old_state.is_none() {
                unsafe {
                    libc::syscall(
                        libc::SYS_renameat2,
                        self.root_fd.as_raw_fd(),
                        c_name.as_ptr(),
                        self.root_fd.as_raw_fd(),
                        state_name.as_ptr(),
                        libc::RENAME_NOREPLACE,
                    ) as i32
                }
            } else {
                unsafe {
                    libc::renameat(
                        self.root_fd.as_raw_fd(),
                        c_name.as_ptr(),
                        self.root_fd.as_raw_fd(),
                        state_name.as_ptr(),
                    )
                }
            };
            if renamed != 0 {
                return Err(io_error(
                    StoreIoOperation::Rename,
                    io::Error::last_os_error(),
                ));
            }
            test_fault!(self, StateDirectorySync);
            self.root_fd
                .sync_all()
                .map_err(|_| StoreError::DurabilityIndeterminate)?;
            Ok(())
        })();
        if write_result.is_err()
            && at_identity_optional(&self.root_fd, STATE_FILE)
                .ok()
                .flatten()
                != Some(temp_identity)
        {
            self.unlink_owned_temp(&name, temp_identity);
        }
        write_result
    }

    fn create_state_temp(&self) -> Result<(String, File), StoreError> {
        for _ in 0..128 {
            let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let name = format!(".state.{}.{}.tmp", std::process::id(), sequence);
            match openat_file(
                &self.root_fd,
                &name,
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC,
                PRIVATE_FILE_MODE,
            ) {
                Ok(file) => {
                    fchmod_exact(&file, PRIVATE_FILE_MODE)?;
                    return Ok((name, file));
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(io_error(StoreIoOperation::Create, error)),
            }
        }
        Err(StoreError::Io {
            operation: StoreIoOperation::Create,
            kind: io::ErrorKind::AlreadyExists,
        })
    }

    fn unlink_owned_temp(&self, name: &str, identity: FileIdentity) {
        if at_identity_optional(&self.root_fd, name).ok().flatten() == Some(identity) {
            if let Ok(name) = cstring(name) {
                unsafe {
                    libc::unlinkat(self.root_fd.as_raw_fd(), name.as_ptr(), 0);
                }
            }
        }
    }

    fn read_state_file_only(&self) -> Result<StateRead, StoreError> {
        let file = openat_file(
            &self.root_fd,
            STATE_FILE,
            libc::O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC,
            0,
        )
        .map_err(|e| classify_open_error(e, StoreIoOperation::Read))?;
        verify_private_file(&file, self.uid, PRIVATE_FILE_MODE)?;
        let identity = file_identity(&file)?;
        let bytes = read_bounded(&file, MAX_STATE_BYTES, StoreIoOperation::Read)?;
        Ok(StateRead {
            graph: GraphSnapshot::new(),
            file: identity,
            digest: sha256(&bytes),
        })
    }

    fn begin_removal_locked(
        &self,
        expected_revision: u64,
        plan_id: Id,
        source_id: Id,
    ) -> Result<GraphSnapshot, StoreError> {
        let current = self.read_state_locked()?;
        if let Some(existing) = current.graph.staged_removals.get(&plan_id) {
            if existing.source_id != source_id {
                return Err(StoreError::Conflict {
                    expected: expected_revision,
                    current: current.graph.revision,
                });
            }
            return Ok(current.graph);
        }
        self.check_revision(expected_revision, current.graph.revision)?;
        if let Some(pending) = current
            .graph
            .staged_removals
            .values()
            .find(|plan| plan.source_id == source_id && plan.stage != RemovalStage::Complete)
        {
            return Err(StoreError::PendingRemoval {
                source_id: pending.source_id.clone(),
            });
        }
        if !current.graph.sources.contains_key(&source_id) {
            return Err(StoreError::NotFound);
        }
        let references = source_references(&current.graph, &source_id);
        if !references.is_empty() {
            return Err(StoreError::SourceInUse {
                source_id,
                references,
            });
        }
        let candidates = source_removal_candidates(&current.graph, &source_id);
        let mut candidate = current.graph.clone();
        candidate.sources.remove(&source_id);
        candidate
            .nodes
            .retain(|_, node| node.source_id != source_id);
        candidate.staged_removals.insert(
            plan_id.clone(),
            super::model::StagedRemoval {
                schema_version: super::model::SCHEMA_VERSION,
                plan_id: plan_id.clone(),
                source_id: source_id.clone(),
                credential_metadata: candidates,
                removed_credential_refs: BTreeSet::new(),
                stage: RemovalStage::BlobPending,
            },
        );
        reserve_id(&mut candidate, plan_id, EntityIdKind::RemovalPlan)?;
        candidate.revision = current
            .graph
            .revision
            .checked_add(1)
            .ok_or(StoreError::CorruptState)?;
        self.publish_candidate_locked(&current, &candidate, Vec::new(), true)
    }

    fn finish_removal_locked(&self, plan_id: &Id) -> Result<GraphSnapshot, StoreError> {
        let current = self.read_state_locked()?;
        self.finish_removal_locked_with_state(current, plan_id)
    }

    fn finish_removal_locked_with_state(
        &self,
        current: StateRead,
        plan_id: &Id,
    ) -> Result<GraphSnapshot, StoreError> {
        let plan = current
            .graph
            .staged_removals
            .get(plan_id)
            .ok_or(StoreError::NotFound)?;
        if plan.stage == RemovalStage::Complete {
            return Ok(current.graph);
        }
        if plan.stage != RemovalStage::BlobPending {
            return Err(StoreError::PendingRemoval {
                source_id: plan.source_id.clone(),
            });
        }
        let metadata = plan.credential_metadata.clone();
        for item in &metadata {
            if credential_is_referenced(&current.graph, &item.credential_ref) {
                return Err(StoreError::PendingRemoval {
                    source_id: plan.source_id.clone(),
                });
            }
        }
        let mut candidate = current.graph.clone();
        for item in &metadata {
            candidate.credentials.remove(&item.credential_ref);
        }
        {
            let plan = candidate
                .staged_removals
                .get_mut(plan_id)
                .ok_or(StoreError::CorruptState)?;
            for item in &metadata {
                plan.removed_credential_refs
                    .insert(item.credential_ref.clone());
            }
            plan.stage = RemovalStage::Complete;
        }
        candidate.revision = current
            .graph
            .revision
            .checked_add(1)
            .ok_or(StoreError::CorruptState)?;
        diagnose_transition(&current.graph, &candidate)?;
        GraphSnapshot::validate_transition(&current.graph, &candidate).map_err(map_model_error)?;
        candidate.validate().map_err(map_model_error)?;
        let encoded = encode_graph(&candidate)?;

        let mut verified = Vec::with_capacity(metadata.len());
        for item in &metadata {
            match self.read_blob_verified_with_identity(item) {
                Ok((_, identity, stamp, file)) => verified.push((
                    item.credential_ref.clone(),
                    Some(identity),
                    Some(stamp),
                    Some(file),
                )),
                Err(StoreError::NotFound) => {
                    verified.push((item.credential_ref.clone(), None, None, None))
                }
                Err(error) => return Err(error),
            }
        }
        // The full set is hashed before any unlink. Reopen each exact path and recheck
        // identity, metadata stamp, and digest immediately before unlinking it.
        self.verify_bindings()?;
        let state_before_unlink = self.read_state_file_only()?;
        if state_before_unlink.file != current.file || state_before_unlink.digest != current.digest
        {
            return Err(StoreError::UnsafeFilesystem);
        }
        let mut rechecked = Vec::with_capacity(verified.len());
        for (credential_ref, identity, stamp, validated_file) in verified {
            let (Some(identity), Some(stamp), Some(validated_file)) =
                (identity, stamp, validated_file)
            else {
                if at_identity_optional(&self.credentials_fd, &blob_name(&credential_ref))?
                    .is_some()
                {
                    return Err(StoreError::UnsafeFilesystem);
                }
                rechecked.push((credential_ref, None));
                continue;
            };
            let validated_metadata = validated_file
                .metadata()
                .map_err(|e| io_error(StoreIoOperation::Stat, e))?;
            if file_identity(&validated_file)? != identity
                || file_stamp(&validated_metadata) != stamp
            {
                return Err(StoreError::UnsafeFilesystem);
            }
            let item = metadata
                .iter()
                .find(|item| &item.credential_ref == &credential_ref)
                .ok_or(StoreError::CorruptState)?;
            let (bytes, path_identity, path_stamp, path_file) =
                self.read_blob_verified_with_identity(item)?;
            drop(bytes);
            if path_identity != identity || path_stamp != stamp {
                return Err(StoreError::UnsafeFilesystem);
            }
            rechecked.push((
                credential_ref,
                Some((identity, stamp, validated_file, path_file)),
            ));
        }
        // Do not unlink until every pathname has passed the second full validation.
        // Keep a final, per-path guard immediately before every unlink as well.
        for (credential_ref, file_state) in rechecked {
            if let Some((identity, stamp, validated_file, path_file)) = file_state {
                self.verify_bindings()?;
                let validated_metadata = validated_file
                    .metadata()
                    .map_err(|e| io_error(StoreIoOperation::Stat, e))?;
                let path_metadata = path_file
                    .metadata()
                    .map_err(|e| io_error(StoreIoOperation::Stat, e))?;
                let name_string = blob_name(&credential_ref);
                if file_identity(&validated_file)? != identity
                    || file_stamp(&validated_metadata) != stamp
                    || file_identity(&path_file)? != identity
                    || file_stamp(&path_metadata) != stamp
                    || at_identity(&self.credentials_fd, &name_string, false)? != identity
                {
                    return Err(StoreError::UnsafeFilesystem);
                }
                verify_private_file(&path_file, self.uid, PRIVATE_FILE_MODE)?;
                let name = cstring(&blob_name(&credential_ref))?;
                if unsafe { libc::unlinkat(self.credentials_fd.as_raw_fd(), name.as_ptr(), 0) } != 0
                {
                    let error = io::Error::last_os_error();
                    if error.kind() == io::ErrorKind::NotFound {
                        continue;
                    }
                    return Err(io_error(StoreIoOperation::Unlink, error));
                }
                test_fault!(self, AfterBlobUnlink);
            } else if at_identity_optional(&self.credentials_fd, &blob_name(&credential_ref))?
                .is_some()
            {
                return Err(StoreError::UnsafeFilesystem);
            }
        }
        self.credentials_fd
            .sync_all()
            .map_err(|e| io_error(StoreIoOperation::Sync, e))?;

        self.publish_locked(Some(&current), &encoded)?;
        Ok(candidate)
    }
}

#[cfg(test)]
mod fault_tests;

struct StoreLockGuard {
    file: File,
}

impl Drop for StoreLockGuard {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

fn validate_root_argument(path: &Path) -> Result<PathBuf, StoreError> {
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(StoreError::UnsafeFilesystem);
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::CurDir | Component::Prefix(_)
        )
    }) {
        return Err(StoreError::UnsafeFilesystem);
    }
    Ok(path.to_path_buf())
}

fn open_absolute_directory(path: &Path) -> Result<File, StoreError> {
    if !path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::CurDir | Component::Prefix(_)
            )
        })
    {
        return Err(StoreError::UnsafeFilesystem);
    }
    let mut current = open_path_root()?;
    let parts: Vec<_> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_os_string()),
            Component::RootDir => None,
            _ => None,
        })
        .collect();
    let mut traversed = PathBuf::from("/");
    for (index, part) in parts.iter().enumerate() {
        let name = cstring_bytes(part.as_bytes())?;
        let next_fd = unsafe {
            libc::openat(
                current.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC,
            )
        };
        if next_fd < 0 {
            return Err(classify_open_error(
                io::Error::last_os_error(),
                StoreIoOperation::Open,
            ));
        }
        let next = unsafe { File::from_raw_fd(next_fd) };
        traversed.push(&part);
        let metadata = next
            .metadata()
            .map_err(|e| io_error(StoreIoOperation::Stat, e))?;
        if !metadata.file_type().is_dir() {
            return Err(StoreError::UnsafeFilesystem);
        }
        if index + 1 < parts.len() {
            verify_safe_ancestor(&metadata, &traversed)?;
        }
        current = next;
    }
    Ok(current)
}

fn open_path_root() -> Result<File, StoreError> {
    let fd = unsafe {
        libc::open(
            c"/".as_ptr(),
            libc::O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io_error(StoreIoOperation::Open, io::Error::last_os_error()));
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn validate_ancestor_chain(path: &Path) -> Result<(), StoreError> {
    let parent = path.parent().ok_or(StoreError::UnsafeFilesystem)?;
    let fd = open_absolute_directory(parent)?;
    let metadata = fd
        .metadata()
        .map_err(|e| io_error(StoreIoOperation::Stat, e))?;
    verify_safe_ancestor(&metadata, parent)
}

fn verify_safe_ancestor(metadata: &fs::Metadata, path: &Path) -> Result<(), StoreError> {
    if !metadata.file_type().is_dir() {
        return Err(StoreError::UnsafeFilesystem);
    }
    let mode = metadata.mode() & 0o7777;
    let sticky_tmp = path == Path::new("/tmp")
        && metadata.uid() == 0
        && mode & 0o1000 != 0
        && mode & 0o0002 != 0;
    let uid = unsafe { libc::geteuid() };
    if metadata.uid() != 0 && metadata.uid() != uid {
        return Err(StoreError::UnsafeFilesystem);
    }
    if !sticky_tmp && mode & 0o0022 != 0 {
        return Err(StoreError::UnsafeFilesystem);
    }
    Ok(())
}

fn openat_file(dir: &File, name: &str, flags: i32, mode: u32) -> io::Result<File> {
    let name =
        CString::new(name.as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags, mode as libc::mode_t) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn cstring(name: &str) -> Result<CString, StoreError> {
    CString::new(name.as_bytes()).map_err(|_| StoreError::UnsafeFilesystem)
}

fn cstring_bytes(name: &[u8]) -> Result<CString, StoreError> {
    CString::new(name).map_err(|_| StoreError::UnsafeFilesystem)
}

fn verify_directory(file: &File, uid: u32, expected_mode: u32) -> Result<(), StoreError> {
    let metadata = file
        .metadata()
        .map_err(|e| io_error(StoreIoOperation::Stat, e))?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != uid
        || metadata.mode() & 0o7777 != expected_mode
    {
        return Err(StoreError::UnsafeFilesystem);
    }
    Ok(())
}

fn verify_private_file(file: &File, uid: u32, expected_mode: u32) -> Result<(), StoreError> {
    let metadata = file
        .metadata()
        .map_err(|e| io_error(StoreIoOperation::Stat, e))?;
    if !metadata.file_type().is_file()
        || metadata.uid() != uid
        || metadata.mode() & 0o7777 != expected_mode
        || metadata.nlink() != 1
    {
        return Err(StoreError::UnsafeFilesystem);
    }
    Ok(())
}

fn file_stamp(metadata: &fs::Metadata) -> FileStamp {
    FileStamp {
        size: metadata.size(),
        mtime_seconds: metadata.mtime(),
        mtime_nanoseconds: metadata.mtime_nsec(),
        ctime_seconds: metadata.ctime(),
        ctime_nanoseconds: metadata.ctime_nsec(),
        mode: metadata.mode(),
        uid: metadata.uid(),
        gid: metadata.gid(),
        links: metadata.nlink(),
    }
}

fn fchmod_exact(file: &File, mode: u32) -> Result<(), StoreError> {
    if unsafe { libc::fchmod(file.as_raw_fd(), mode as libc::mode_t) } != 0 {
        return Err(io_error(
            StoreIoOperation::Write,
            io::Error::last_os_error(),
        ));
    }
    Ok(())
}

fn file_identity(file: &File) -> Result<FileIdentity, StoreError> {
    let metadata = file
        .metadata()
        .map_err(|e| io_error(StoreIoOperation::Stat, e))?;
    Ok(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

fn at_identity(dir: &File, name: &str, directory: bool) -> Result<FileIdentity, StoreError> {
    let c_name = cstring(name)?;
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe {
        libc::fstatat(
            dir.as_raw_fd(),
            c_name.as_ptr(),
            &mut stat,
            AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::NotFound {
            return Err(StoreError::UnsafeFilesystem);
        }
        return Err(io_error(StoreIoOperation::Stat, error));
    }
    let is_directory = (stat.st_mode & libc::S_IFMT) == libc::S_IFDIR;
    let is_file = (stat.st_mode & libc::S_IFMT) == libc::S_IFREG;
    if (directory && !is_directory) || (!directory && !is_file) {
        return Err(StoreError::UnsafeFilesystem);
    }
    Ok(FileIdentity {
        device: stat.st_dev as u64,
        inode: stat.st_ino as u64,
    })
}

fn at_identity_optional(dir: &File, name: &str) -> Result<Option<FileIdentity>, StoreError> {
    let c_name = cstring(name)?;
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe {
        libc::fstatat(
            dir.as_raw_fd(),
            c_name.as_ptr(),
            &mut stat,
            AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::NotFound {
            return Ok(None);
        }
        return Err(io_error(StoreIoOperation::Stat, error));
    }
    Ok(Some(FileIdentity {
        device: stat.st_dev as u64,
        inode: stat.st_ino as u64,
    }))
}

fn read_bounded(
    file: &File,
    limit: usize,
    operation: StoreIoOperation,
) -> Result<Vec<u8>, StoreError> {
    let length = file
        .metadata()
        .map_err(|e| io_error(StoreIoOperation::Stat, e))?
        .len();
    if length > limit as u64 {
        return Err(StoreError::CorruptState);
    }
    let mut reader = file.take(limit as u64 + 1);
    let mut output = Vec::with_capacity(length as usize);
    reader
        .read_to_end(&mut output)
        .map_err(|e| io_error(operation, e))?;
    if output.len() > limit {
        return Err(StoreError::CorruptState);
    }
    Ok(output)
}

fn encode_graph(graph: &GraphSnapshot) -> Result<Vec<u8>, StoreError> {
    let bytes = serde_json::to_vec(graph).map_err(|_| StoreError::CorruptState)?;
    if bytes.len() > MAX_STATE_BYTES {
        return Err(StoreError::CorruptState);
    }
    Ok(bytes)
}

fn map_model_error(error: ModelError) -> StoreError {
    if error == ModelError::UnsupportedSchemaVersion {
        StoreError::UnsupportedSchema
    } else {
        StoreError::InvalidModel(error)
    }
}

fn decode_graph(bytes: &[u8]) -> Result<GraphSnapshot, StoreError> {
    let graph: GraphSnapshot = match serde_json::from_slice(bytes) {
        Ok(graph) => graph,
        Err(_) => {
            let value: serde_json::Value =
                serde_json::from_slice(bytes).map_err(|_| StoreError::CorruptState)?;
            if has_unsupported_schema(&value) {
                return Err(StoreError::UnsupportedSchema);
            }
            return Err(StoreError::CorruptState);
        }
    };
    graph.validate().map_err(|error| match error {
        ModelError::UnsupportedSchemaVersion => StoreError::UnsupportedSchema,
        _ => StoreError::CorruptState,
    })?;
    Ok(graph)
}

fn has_unsupported_schema(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            map.get("schema_version").is_some_and(|version| {
                version.as_u64() != Some(super::model::SCHEMA_VERSION as u64)
            }) || map.values().any(has_unsupported_schema)
        }
        serde_json::Value::Array(values) => values.iter().any(has_unsupported_schema),
        _ => false,
    }
}

fn index_materials(
    materials: Vec<CredentialMaterial>,
) -> Result<BTreeMap<Id, CredentialMaterial>, StoreError> {
    let mut indexed = BTreeMap::new();
    for material in materials {
        let id = material.credential_ref.clone();
        if indexed.insert(id, material).is_some() {
            return Err(StoreError::CredentialMismatch);
        }
    }
    Ok(indexed)
}

pub(crate) fn make_metadata(
    credential_ref: Id,
    owner_source_id: Id,
    bytes: &[u8],
) -> CredentialMetadata {
    CredentialMetadata {
        schema_version: super::model::SCHEMA_VERSION,
        credential_ref,
        owner_source_id,
        digest_sha256: hex_sha256(bytes),
        size_bytes: bytes.len() as u64,
    }
}

fn check_material(
    metadata: &CredentialMetadata,
    material: &CredentialMaterial,
) -> Result<(), StoreError> {
    if metadata.credential_ref != material.credential_ref
        || metadata.size_bytes != material.bytes.len() as u64
        || metadata.digest_sha256 != hex_sha256(&material.bytes)
    {
        return Err(StoreError::CredentialMismatch);
    }
    Ok(())
}

fn ensure_material_metadata(
    old: &GraphSnapshot,
    candidate: &mut GraphSnapshot,
    metadata: &CredentialMetadata,
    materials: &[CredentialMaterial],
    source_id: &Id,
) -> Result<(), StoreError> {
    if let Some(existing) = candidate.credentials.get(&metadata.credential_ref) {
        if existing != metadata {
            return Err(StoreError::CredentialMismatch);
        }
        if old.credentials.contains_key(&metadata.credential_ref)
            && materials
                .iter()
                .any(|material| material.credential_ref == metadata.credential_ref)
        {
            return Err(StoreError::CredentialOccupied {
                credential_ref: metadata.credential_ref.clone(),
            });
        }
        return Ok(());
    }
    let material = materials
        .iter()
        .find(|material| material.credential_ref == metadata.credential_ref)
        .ok_or_else(|| StoreError::MissingCredential {
            credential_ref: metadata.credential_ref.clone(),
        })?;
    let expected = make_metadata(
        metadata.credential_ref.clone(),
        source_id.clone(),
        &material.bytes,
    );
    if &expected != metadata {
        return Err(StoreError::CredentialMismatch);
    }
    reserve_id(
        candidate,
        metadata.credential_ref.clone(),
        EntityIdKind::CredentialRef,
    )?;
    candidate
        .credentials
        .insert(metadata.credential_ref.clone(), metadata.clone());
    Ok(())
}

pub(crate) fn reserve_id(
    graph: &mut GraphSnapshot,
    id: Id,
    kind: EntityIdKind,
) -> Result<(), StoreError> {
    if graph.issued_ids.contains(&id) || graph.issued_id_kinds.contains_key(&id) {
        return Err(StoreError::InvalidModel(ModelError::IdReuse));
    }
    graph.issued_ids.insert(id.clone());
    graph.issued_id_kinds.insert(id, kind);
    Ok(())
}

pub(crate) fn fresh_id(graph: &GraphSnapshot, prefix: &str) -> Result<Id, StoreError> {
    for _ in 0..8 {
        let mut bytes = [0u8; 16];
        let mut filled = 0;
        while filled < bytes.len() {
            let got = unsafe {
                libc::getrandom(bytes[filled..].as_mut_ptr().cast(), bytes.len() - filled, 0)
            };
            if got < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(io_error(StoreIoOperation::Read, error));
            }
            if got == 0 {
                return Err(StoreError::Io {
                    operation: StoreIoOperation::Read,
                    kind: io::ErrorKind::UnexpectedEof,
                });
            }
            filled += got as usize;
        }
        let value = format!("{prefix}-{}", hex(&bytes));
        let id = Id::new(value).map_err(StoreError::InvalidModel)?;
        if !graph.issued_ids.contains(&id) {
            return Ok(id);
        }
    }
    Err(StoreError::Io {
        operation: StoreIoOperation::Create,
        kind: io::ErrorKind::AlreadyExists,
    })
}

fn source_references(graph: &GraphSnapshot, source_id: &Id) -> Vec<Id> {
    let mut references = BTreeSet::new();
    for profile in graph
        .connection_profiles
        .values()
        .filter(|profile| &profile.source_id == source_id)
    {
        references.insert(profile.id.clone());
    }
    for session in graph.sessions.values().filter(|session| {
        session.lifecycle == SessionLifecycle::Active && &session.source_id == source_id
    }) {
        references.insert(session.id.clone());
    }
    references.into_iter().collect()
}

fn credential_is_referenced(graph: &GraphSnapshot, credential_ref: &Id) -> bool {
    graph.sources.values().any(|source| {
        source
            .credential
            .as_ref()
            .is_some_and(|metadata| &metadata.credential_ref == credential_ref)
    }) || graph
        .nodes
        .values()
        .any(|node| node.credential_refs.contains(credential_ref))
}

fn diagnose_transition(old: &GraphSnapshot, new: &GraphSnapshot) -> Result<(), StoreError> {
    for (node_id, old_node) in &old.nodes {
        if new
            .nodes
            .get(node_id)
            .is_some_and(|new_node| new_node != old_node)
        {
            let mut references = BTreeSet::new();
            for profile in old.connection_profiles.values() {
                if matches!(&profile.node_selection, super::model::NodeSelection::Pinned { node } if &node.node_id == node_id)
                {
                    references.insert(profile.id.clone());
                }
            }
            for session in old.sessions.values().filter(|session| {
                session.lifecycle == SessionLifecycle::Active && &session.node_id == node_id
            }) {
                references.insert(session.id.clone());
            }
            return Err(StoreError::ImmutableNode {
                node_id: node_id.clone(),
                references: references.into_iter().collect(),
            });
        }
    }
    for source_id in old
        .sources
        .keys()
        .filter(|id| !new.sources.contains_key(*id))
    {
        let references = source_references(old, source_id);
        if !references.is_empty() {
            return Err(StoreError::SourceInUse {
                source_id: source_id.clone(),
                references,
            });
        }
    }
    Ok(())
}

fn blob_name(id: &Id) -> String {
    format!("{}.blob", id.as_str())
}

fn io_error(operation: StoreIoOperation, error: io::Error) -> StoreError {
    StoreError::Io {
        operation,
        kind: error.kind(),
    }
}

fn classify_open_error(error: io::Error, operation: StoreIoOperation) -> StoreError {
    if matches!(
        error.raw_os_error(),
        Some(libc::ELOOP) | Some(libc::ENOTDIR)
    ) {
        StoreError::UnsafeFilesystem
    } else if error.kind() == io::ErrorKind::NotFound {
        StoreError::NotFound
    } else {
        io_error(operation, error)
    }
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn hex_sha256(bytes: &[u8]) -> String {
    hex(&sha256(bytes))
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

//! Field names whose values stay inside the private node definition.

#[allow(dead_code)]
pub const SECRET_FIELDS: &[&str] = &[
    "uuid",
    "password",
    "private-key",
    "pre-shared-key",
    "obfs-password",
    "auth",
    "username",
    "token",
    "certificate",
];

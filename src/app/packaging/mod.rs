//! Способ запуска и то, что CM о нём может обещать.

pub mod detect;
pub mod policy;

pub use detect::{Packaging, detect, read_head};
pub use policy::{Limit, Reason, Support, limit_token, reason_token, support};

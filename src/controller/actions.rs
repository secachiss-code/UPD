//! Действия polkit по классам операций. Вызов `pkcheck` живёт только здесь.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::peer::PeerIdentity;
use super::protocol::{ControlError, OpClass};
use crate::common::test_mode;
use crate::common::{CapturePolicy, capture_with_policy};
use crate::helper::ACTION_STATUS;

pub const ACTION_WORKER: &str = "io.github.cm.worker";
pub const ACTION_NET: &str = "io.github.cm.net";
pub const ACTION_APP: &str = "io.github.cm.app";

pub fn action_for(class: OpClass) -> &'static str {
    match class {
        OpClass::Status => ACTION_STATUS,
        OpClass::Worker => ACTION_WORKER,
        OpClass::Net => ACTION_NET,
        OpClass::App => ACTION_APP,
    }
}

pub trait Authorizer: Send + Sync {
    fn authorize(&self, peer: &PeerIdentity, action: &str) -> Result<(), ControlError>;
}

pub struct PolkitAuthorizer;

impl Authorizer for PolkitAuthorizer {
    fn authorize(&self, peer: &PeerIdentity, action: &str) -> Result<(), ControlError> {
        if peer.uid == 0 || (test_mode() && std::env::var("CM_HELPER_ALLOW").as_deref() == Ok("1"))
        {
            return Ok(());
        }
        match pkcheck(peer.pid, peer.start_time, peer.uid, action) {
            PkResult::Allowed => Ok(()),
            _ => Err(ControlError::Denied),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PkResult {
    Allowed,
    Dismissed,
    NoAgent,
    Denied,
    Failed,
}

/// Единственный запуск `pkcheck`. Субъект — pid, время старта и uid, переданные вызывающим.
pub fn pkcheck(pid: i32, start_time: u64, uid: u32, action: &str) -> PkResult {
    let subject = format!("{pid},{start_time},{uid}");
    let mut cmd = Command::new("pkcheck");
    cmd.args([
        "--action-id",
        action,
        "--process",
        &subject,
        "--allow-user-interaction",
    ])
    .stdin(Stdio::null());
    let mut policy = CapturePolicy::background(Some(64 << 10));
    policy.deadline = Instant::now() + Duration::from_secs(300);
    match capture_with_policy(&mut cmd, policy) {
        Ok(output) => match output.status.code() {
            Some(0) => PkResult::Allowed,
            Some(3) => PkResult::Dismissed,
            Some(2) => PkResult::NoAgent,
            _ => PkResult::Denied,
        },
        Err(_) => PkResult::Failed,
    }
}

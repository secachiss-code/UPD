//! `mihomo -t` inside a caller-supplied sandbox.
//!
//! Classification keeps the fixed code and drops stderr, stdout, and the config.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use crate::common::{CapturePolicy, capture_with_policy};

/// `-t` only parses; a slower run means the core went for network data (GEOIP/GEOSITE).
const VALIDATION_TIMEOUT: Duration = Duration::from_secs(20);
const OUTPUT_MAX: u64 = 64 << 10;

/// Why `mihomo -t` did not accept a file. Display and Debug are the variant name.
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum CoreValidationCode {
    Syntax,
    UnknownField,
    MissingField,
    InvalidValue,
    Rejected,
    Unavailable,
}

impl CoreValidationCode {
    fn name(self) -> &'static str {
        match self {
            Self::Syntax => "Syntax",
            Self::UnknownField => "UnknownField",
            Self::MissingField => "MissingField",
            Self::InvalidValue => "InvalidValue",
            Self::Rejected => "Rejected",
            Self::Unavailable => "Unavailable",
        }
    }
}

impl std::fmt::Display for CoreValidationCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl std::fmt::Debug for CoreValidationCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl std::error::Error for CoreValidationCode {}

/// Write `config` into `sandbox` as mode 0600 and run `binary -t -d sandbox -f file`.
pub fn validate_file(
    binary: &Path,
    sandbox: &Path,
    config: &[u8],
) -> Result<(), CoreValidationCode> {
    validate_as(binary, sandbox, sandbox, config, None)
}

/// The same check for a core that runs as another user: the candidate is written into
/// `sandbox` (a directory the manager keeps), handed to that user, and the core checks it
/// with the user's privileges and `home` as its directory. The core never parses a
/// caller's config with the manager's privileges.
pub fn validate_as(
    binary: &Path,
    sandbox: &Path,
    home: &Path,
    config: &[u8],
    run_as: Option<&crate::controller::drop::RunAs>,
) -> Result<(), CoreValidationCode> {
    let path = sandbox.join("config.json");
    let owner = run_as.map(|run_as| crate::core::instance::InstanceOwner {
        uid: run_as.uid,
        gid: run_as.gid,
    });
    // A planted symlink in the sandbox must not redirect the write.
    crate::core::process::write_for(&path, config, owner)
        .map_err(|_| CoreValidationCode::Unavailable)?;
    let mut policy = CapturePolicy::background(Some(OUTPUT_MAX));
    policy.deadline = Instant::now() + VALIDATION_TIMEOUT;
    policy.stderr_max = OUTPUT_MAX;
    let mut command = Command::new(binary);
    if run_as.is_some() {
        command.env_clear();
    }
    command
        .arg("-t")
        .arg("-d")
        .arg(home)
        .arg("-f")
        .arg(&path)
        .env("LC_ALL", "C");
    if let Some(run_as) = run_as {
        crate::controller::drop::drop_pre_exec(
            &mut command,
            run_as.clone(),
            &[],
            crate::controller::harden::ChildLimits::default(),
        );
    }
    let output =
        capture_with_policy(&mut command, policy).map_err(|_| CoreValidationCode::Unavailable)?;
    if output.status.success() {
        return Ok(());
    }
    Err(classify(&output.stderr, &output.stdout))
}

fn classify(stderr: &[u8], stdout: &[u8]) -> CoreValidationCode {
    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(stderr));
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(stdout));
    let lower = text.to_ascii_lowercase();
    drop(text);
    if lower.contains("missing") {
        return CoreValidationCode::MissingField;
    }
    if lower.contains("unknown") || lower.contains("not support") || lower.contains("unsupport") {
        return CoreValidationCode::UnknownField;
    }
    if lower.contains("unmarshal")
        || lower.contains("syntax")
        || lower.contains("invalid character")
        || lower.contains("did not find expected")
    {
        return CoreValidationCode::Syntax;
    }
    if lower.contains("invalid") {
        return CoreValidationCode::InvalidValue;
    }
    CoreValidationCode::Rejected
}

//! `mihomo -t` inside a caller-supplied sandbox.
//!
//! Classification keeps the fixed code and drops stderr, stdout, and the config.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::Command;

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
    let path = sandbox.join("config.json");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
        .map_err(|_| CoreValidationCode::Unavailable)?;
    file.write_all(config)
        .map_err(|_| CoreValidationCode::Unavailable)?;
    drop(file);
    let output = Command::new(binary)
        .arg("-t")
        .arg("-d")
        .arg(sandbox)
        .arg("-f")
        .arg(&path)
        .output()
        .map_err(|_| CoreValidationCode::Unavailable)?;
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

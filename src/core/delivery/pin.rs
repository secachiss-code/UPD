//! Закреплённые версии из I05-PIN.md. Значения не выводятся и не подменяются.

use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoreKind {
    Mihomo,
    Xray,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Archive {
    Gzip,
    Zip { member: &'static str },
}

pub struct Pin {
    pub kind: CoreKind,
    pub version: &'static str,
    pub url: &'static str,
    pub archive: Archive,
    pub archive_sha256: &'static str,
    pub binary_sha256: &'static str,
    pub max_archive: u64,
    pub max_binary: u64,
    pub version_marker: &'static str,
}

const MAX_ARCHIVE: u64 = 64 << 20;
const MAX_BINARY: u64 = 128 << 20;

pub static PINS: &[Pin] = &[
    Pin {
        kind: CoreKind::Mihomo,
        version: "v1.19.32",
        url: "https://github.com/MetaCubeX/mihomo/releases/download/v1.19.32/mihomo-linux-amd64-compatible-v1.19.32.gz",
        archive: Archive::Gzip,
        archive_sha256: "ba3ce607747a07f948fc35780e108a4a7c7f552a38b9bd4d115f313ebcb89c20",
        binary_sha256: "7a0d59da2e678d56c899a3db996a2ad8963286c4f3634b0451435db248f13fa1",
        max_archive: MAX_ARCHIVE,
        max_binary: MAX_BINARY,
        version_marker: "v1.19.32",
    },
    Pin {
        kind: CoreKind::Xray,
        version: "v26.3.27",
        url: "https://github.com/XTLS/Xray-core/releases/download/v26.3.27/Xray-linux-64.zip",
        archive: Archive::Zip { member: "xray" },
        archive_sha256: "23cd9af937744d97776ee35ecad4972cf4b2109d1e0fe6be9930467608f7c8ae",
        binary_sha256: "8255dd939c34cf966cc91517b6324dd3c8d0bcf49ffac8beca049a38c46845ed",
        max_archive: MAX_ARCHIVE,
        max_binary: MAX_BINARY,
        version_marker: "Xray 26.3.27",
    },
];

pub fn pin(kind: CoreKind) -> &'static Pin {
    PINS.iter()
        .find(|item| item.kind == kind)
        .expect("both kinds are pinned")
}

pub fn verify_sha256(bytes: &[u8], expected_hex: &str) -> Result<(), DeliveryError> {
    let expected = parse_hex(expected_hex).ok_or(DeliveryError::HashMismatch)?;
    let actual = Sha256::digest(bytes);
    if actual.as_slice() == expected {
        Ok(())
    } else {
        Err(DeliveryError::HashMismatch)
    }
}

fn parse_hex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryError {
    HashMismatch,
    TooLarge,
    BadArchive,
    MemberMissing,
    Fetch,
    VersionMismatch,
    ConfigRejected,
    Io,
    NothingToRollBack,
}

impl std::fmt::Display for DeliveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::HashMismatch => "hash mismatch",
            Self::TooLarge => "archive or binary is too large",
            Self::BadArchive => "archive is not usable",
            Self::MemberMissing => "archive member is missing",
            Self::Fetch => "download failed",
            Self::VersionMismatch => "binary version does not match the pin",
            Self::ConfigRejected => "candidate rejected the current config",
            Self::Io => "install failed",
            Self::NothingToRollBack => "nothing to roll back",
        })
    }
}

impl std::error::Error for DeliveryError {}

//! K01–K04: pinned cores, bounded unpacking, verified fetch, atomic install and rollback.

mod pack_support;

use std::cell::RefCell;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

use cm::common::contract_fixtures::TempDirGuard;
use cm::core::delivery::{
    Archive, CoreKind, DeliveryError, Download, PINS, Pin, TransportDownload, current, install,
    obtain, pin, rollback, unpack_gzip, unpack_zip_member, verify_sha256,
};
use flate2::Compression;
use flate2::write::{DeflateEncoder, GzEncoder};
use pack_support::{Xorshift, mutate, sha256_hex};

#[test]
fn k01_pins_match_the_record() {
    let mihomo = pin(CoreKind::Mihomo);
    assert_eq!(
        mihomo.archive_sha256,
        "ba3ce607747a07f948fc35780e108a4a7c7f552a38b9bd4d115f313ebcb89c20"
    );
    assert_eq!(
        mihomo.binary_sha256,
        "7a0d59da2e678d56c899a3db996a2ad8963286c4f3634b0451435db248f13fa1"
    );
    assert_eq!(mihomo.archive, Archive::Gzip);
    let xray = pin(CoreKind::Xray);
    assert_eq!(
        xray.archive_sha256,
        "23cd9af937744d97776ee35ecad4972cf4b2109d1e0fe6be9930467608f7c8ae"
    );
    assert_eq!(
        xray.binary_sha256,
        "8255dd939c34cf966cc91517b6324dd3c8d0bcf49ffac8beca049a38c46845ed"
    );
    assert_eq!(xray.archive, Archive::Zip { member: "xray" });
    assert_eq!(PINS.len(), 2);
    let record = include_str!("../docs/design/cm-network-manager/I05-PIN.md");
    for item in PINS {
        assert!(item.url.starts_with("https://github.com/"));
        assert!(item.url.contains(item.version));
        assert!(record.contains(item.archive_sha256));
        assert!(record.contains(item.binary_sha256));
        assert!(item.max_archive <= 64 << 20 && item.max_binary <= 128 << 20);
    }
}

#[test]
fn k01_hash_check() {
    let hex = "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a";
    assert_eq!(verify_sha256(b"{}", hex), Ok(()));
    assert_eq!(verify_sha256(b"{}", &hex.to_uppercase()), Ok(()));
    let changed = format!("5{}", &hex[1..]);
    assert_eq!(
        verify_sha256(b"{}", &changed),
        Err(DeliveryError::HashMismatch)
    );
    assert_eq!(
        verify_sha256(b"{}", &hex[1..]),
        Err(DeliveryError::HashMismatch)
    );
    assert_eq!(verify_sha256(b"{}", ""), Err(DeliveryError::HashMismatch));
}

#[test]
fn k01_local_cores_are_the_pinned_ones() {
    for (variable, kind) in [
        ("CM_TEST_MIHOMO", CoreKind::Mihomo),
        ("CM_TEST_XRAY", CoreKind::Xray),
    ] {
        let Some(path) = std::env::var_os(variable) else {
            eprintln!("K01 SKIPPED: {variable} not set");
            continue;
        };
        let bytes = fs::read(path).unwrap();
        assert_eq!(
            verify_sha256(&bytes, pin(kind).binary_sha256),
            Ok(()),
            "{variable}"
        );
    }
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

#[test]
fn k02_gzip_is_bounded() {
    let data: Vec<u8> = (0..1000u32).map(|value| (value * 7) as u8).collect();
    let archive = gzip(&data);
    assert_eq!(unpack_gzip(&archive, 1000).unwrap(), data);
    assert_eq!(unpack_gzip(&archive, 999), Err(DeliveryError::TooLarge));
    assert_eq!(
        unpack_gzip(&archive[..archive.len() - 10], 1000),
        Err(DeliveryError::BadArchive)
    );
    let bomb = gzip(&vec![0u8; 64 << 20]);
    assert!(bomb.len() < 1 << 20);
    assert_eq!(unpack_gzip(&bomb, 1 << 20), Err(DeliveryError::TooLarge));
    assert_eq!(unpack_gzip(b"", 10), Err(DeliveryError::BadArchive));
}

struct Entry<'a> {
    name: &'a str,
    data: &'a [u8],
    method: u16,
    flags: u16,
    crc: Option<u32>,
    size: Option<u32>,
}

fn entry<'a>(name: &'a str, data: &'a [u8]) -> Entry<'a> {
    Entry {
        name,
        data,
        method: 8,
        flags: 0,
        crc: None,
        size: None,
    }
}

fn zip(entries: &[Entry]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for item in entries {
        let packed = if item.method == 0 {
            item.data.to_vec()
        } else {
            let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(item.data).unwrap();
            encoder.finish().unwrap()
        };
        let mut crc = flate2::Crc::new();
        crc.update(item.data);
        let crc = item.crc.unwrap_or(crc.sum());
        let size = item.size.unwrap_or(item.data.len() as u32);
        let offset = out.len() as u32;
        let mut header = Vec::new();
        header.extend_from_slice(&20u16.to_le_bytes());
        header.extend_from_slice(&item.flags.to_le_bytes());
        header.extend_from_slice(&item.method.to_le_bytes());
        header.extend_from_slice(&[0; 4]);
        header.extend_from_slice(&crc.to_le_bytes());
        header.extend_from_slice(&(packed.len() as u32).to_le_bytes());
        header.extend_from_slice(&size.to_le_bytes());
        header.extend_from_slice(&(item.name.len() as u16).to_le_bytes());
        header.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(b"PK\x03\x04");
        out.extend_from_slice(&header);
        out.extend_from_slice(item.name.as_bytes());
        out.extend_from_slice(&packed);
        central.extend_from_slice(b"PK\x01\x02");
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&header);
        central.extend_from_slice(&[0; 2]);
        central.extend_from_slice(&[0; 8]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(item.name.as_bytes());
    }
    let start = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(b"PK\x05\x06");
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

const BINARY: &[u8] = b"#!/bin/sh\necho core-binary-payload core-binary-payload\n";

#[test]
fn k02_zip_member() {
    let archive = zip(&[entry("LICENSE", b"license text"), entry("xray", BINARY)]);
    assert_eq!(
        unpack_zip_member(&archive, "xray", 1 << 20).unwrap(),
        BINARY
    );
    assert_eq!(
        unpack_zip_member(&archive, "LICENSE", 1 << 20).unwrap(),
        b"license text"
    );
    assert_eq!(
        unpack_zip_member(&archive, "nope", 1 << 20),
        Err(DeliveryError::MemberMissing)
    );
    let stored = zip(&[Entry {
        method: 0,
        ..entry("xray", BINARY)
    }]);
    assert_eq!(unpack_zip_member(&stored, "xray", 1 << 20).unwrap(), BINARY);
    assert_eq!(
        unpack_zip_member(&archive, "xray", BINARY.len() as u64 - 1),
        Err(DeliveryError::TooLarge)
    );
}

#[test]
fn k02_zip_lies_are_refused() {
    let twenty = [7u8; 20];
    let cases = [
        (
            Entry {
                size: Some(10),
                ..entry("xray", &twenty)
            },
            DeliveryError::BadArchive,
        ),
        (
            Entry {
                size: Some(1 << 30),
                ..entry("xray", &twenty)
            },
            DeliveryError::TooLarge,
        ),
        (
            Entry {
                crc: Some(1),
                ..entry("xray", &twenty)
            },
            DeliveryError::BadArchive,
        ),
        (
            Entry {
                flags: 1,
                ..entry("xray", &twenty)
            },
            DeliveryError::BadArchive,
        ),
        (
            Entry {
                method: 12,
                ..entry("xray", &twenty)
            },
            DeliveryError::BadArchive,
        ),
    ];
    for (item, expected) in cases {
        assert_eq!(
            unpack_zip_member(&zip(&[item]), "xray", 1 << 20),
            Err(expected)
        );
    }
    let traversal = zip(&[entry("../xray", BINARY), entry("a/xray", BINARY)]);
    for member in ["xray", "../xray", "a/xray", "", "."] {
        assert_eq!(
            unpack_zip_member(&traversal, member, 1 << 20),
            Err(DeliveryError::MemberMissing),
            "{member:?}"
        );
    }
    let good = zip(&[entry("xray", BINARY)]);
    for broken in [&good[..good.len() - 22], &b""[..], &[0x55u8; 22][..]] {
        assert_eq!(
            unpack_zip_member(broken, "xray", 1 << 20),
            Err(DeliveryError::BadArchive)
        );
    }
}

#[test]
fn k02_zip_mutation_never_panics() {
    let sample = zip(&[entry("LICENSE", b"license text"), entry("xray", BINARY)]);
    let mut rng = Xorshift(1);
    let mut accepted = 0u32;
    for _ in 0..20_000 {
        let bytes = mutate(&sample, &mut rng);
        match unpack_zip_member(&bytes, "xray", 4096) {
            Ok(output) => {
                assert!(output.len() <= 4096);
                accepted += 1;
            }
            Err(error) => assert!(matches!(
                error,
                DeliveryError::BadArchive | DeliveryError::MemberMissing | DeliveryError::TooLarge
            )),
        }
    }
    assert!(accepted < 20_000);
}

#[test]
fn k02_real_xray_archive() {
    let Some(path) = std::env::var_os("CM_TEST_XRAY_ZIP") else {
        eprintln!(
            "K02 NOT_APPLICABLE: CM_TEST_XRAY_ZIP not set (the release archive is not fetched in tests)"
        );
        return;
    };
    let archive = fs::read(path).unwrap();
    let xray = pin(CoreKind::Xray);
    assert_eq!(verify_sha256(&archive, xray.archive_sha256), Ok(()));
    let binary = unpack_zip_member(&archive, "xray", xray.max_binary).unwrap();
    assert_eq!(verify_sha256(&binary, xray.binary_sha256), Ok(()));
}

#[test]
fn k02_real_mihomo_archive() {
    let Some(path) = std::env::var_os("CM_TEST_MIHOMO_GZ") else {
        eprintln!(
            "K02 NOT_APPLICABLE: CM_TEST_MIHOMO_GZ not set (the release archive is not fetched in tests)"
        );
        return;
    };
    let archive = fs::read(path).unwrap();
    let mihomo = pin(CoreKind::Mihomo);
    assert_eq!(verify_sha256(&archive, mihomo.archive_sha256), Ok(()));
    let binary = unpack_gzip(&archive, mihomo.max_binary).unwrap();
    assert_eq!(verify_sha256(&binary, mihomo.binary_sha256), Ok(()));
}

struct Scripted {
    reply: Result<Vec<u8>, DeliveryError>,
    calls: RefCell<Vec<(String, u64)>>,
}

impl Download for Scripted {
    fn get(&self, url: &str, max_bytes: u64) -> Result<Vec<u8>, DeliveryError> {
        self.calls.borrow_mut().push((url.to_owned(), max_bytes));
        self.reply.clone()
    }
}

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

fn test_pin(archive: &[u8], binary: &[u8], version: &'static str) -> Pin {
    Pin {
        kind: CoreKind::Mihomo,
        version,
        url: "https://example.invalid/core.gz",
        archive: Archive::Gzip,
        archive_sha256: leak(sha256_hex(archive)),
        binary_sha256: leak(sha256_hex(binary)),
        max_archive: 4096,
        max_binary: 4096,
        version_marker: version,
    }
}

#[test]
fn k03_only_a_twice_verified_binary_leaves() {
    let archive = gzip(BINARY);
    let good = test_pin(&archive, BINARY, "v1");
    let download = Scripted {
        reply: Ok(archive.clone()),
        calls: RefCell::new(Vec::new()),
    };
    assert_eq!(obtain(&good, &download).unwrap(), BINARY);
    assert_eq!(
        *download.calls.borrow(),
        [("https://example.invalid/core.gz".to_owned(), 4096)]
    );
    // A bomb with a wrong hash is refused before unpacking: the error is the hash, not the size.
    let bomb = Scripted {
        reply: Ok(gzip(&vec![0u8; 8 << 20])),
        calls: RefCell::new(Vec::new()),
    };
    assert_eq!(obtain(&good, &bomb), Err(DeliveryError::HashMismatch));
    let other_binary = test_pin(&archive, b"another binary", "v1");
    assert_eq!(
        obtain(&other_binary, &download),
        Err(DeliveryError::HashMismatch)
    );
    for error in [DeliveryError::Fetch, DeliveryError::TooLarge] {
        let failing = Scripted {
            reply: Err(error),
            calls: RefCell::new(Vec::new()),
        };
        assert_eq!(obtain(&good, &failing), Err(error));
    }
    assert_eq!(
        TransportDownload.get("http://example.invalid/x", 10),
        Err(DeliveryError::Fetch)
    );
}

fn target(root: &Path, link: &str) -> String {
    fs::read_link(root.join("mihomo").join(link))
        .map(|path| path.display().to_string())
        .unwrap_or_default()
}

fn current_is_executable(root: &Path) {
    let link = root.join("mihomo/current");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let meta = fs::metadata(&link).unwrap();
    assert!(meta.is_file());
    assert_eq!(meta.mode() & 0o777, 0o755);
}

#[test]
fn k04_install_and_rollback() {
    let dir = TempDirGuard::new("cm-pack-k04").unwrap();
    let root = dir.path();
    let one = test_pin(b"a", b"binary-one", "v1");
    let two = test_pin(b"a", b"binary-two", "v2");
    let three = test_pin(b"a", b"binary-three", "v3");
    let probe = |path: &Path| -> Result<String, DeliveryError> {
        Ok(format!(
            "core {}",
            String::from_utf8_lossy(&fs::read(path).unwrap())
        ))
    };
    let named = |version: &'static str| {
        move |_: &Path| -> Result<String, DeliveryError> { Ok(format!("core {version}")) }
    };
    let accept = |_: &Path| -> Result<(), DeliveryError> { Ok(()) };
    let _ = probe;
    assert_eq!(
        rollback(root, CoreKind::Mihomo).err(),
        Some(DeliveryError::NothingToRollBack)
    );
    assert!(current(root, CoreKind::Mihomo).is_none());

    let installed = install(root, &one, b"binary-one", &named("v1"), &accept).unwrap();
    assert_eq!(installed.version, "v1");
    assert_eq!(target(root, "current"), "versions/v1/mihomo");
    assert_eq!(target(root, "previous"), "");
    current_is_executable(root);
    assert_eq!(
        rollback(root, CoreKind::Mihomo).err(),
        Some(DeliveryError::NothingToRollBack)
    );

    install(root, &two, b"binary-two", &named("v2"), &accept).unwrap();
    assert_eq!(target(root, "current"), "versions/v2/mihomo");
    assert_eq!(target(root, "previous"), "versions/v1/mihomo");
    assert_eq!(
        fs::read(root.join("mihomo/versions/v1/mihomo")).unwrap(),
        b"binary-one"
    );
    current_is_executable(root);

    // The candidate reports another version: nothing moves and no staging directory stays.
    assert_eq!(
        install(root, &three, b"binary-three", &named("v2"), &accept).err(),
        Some(DeliveryError::VersionMismatch)
    );
    assert_eq!(target(root, "current"), "versions/v2/mihomo");
    assert!(!root.join("mihomo/versions/v3.tmp").exists());
    assert!(!root.join("mihomo/versions/v3").exists());
    let reject = |_: &Path| -> Result<(), DeliveryError> { Err(DeliveryError::ConfigRejected) };
    assert_eq!(
        install(root, &three, b"binary-three", &named("v3"), &reject).err(),
        Some(DeliveryError::ConfigRejected)
    );
    assert_eq!(target(root, "current"), "versions/v2/mihomo");
    assert!(!root.join("mihomo/versions/v3.tmp").exists());
    current_is_executable(root);

    assert_eq!(rollback(root, CoreKind::Mihomo).unwrap().version, "v1");
    assert_eq!(target(root, "current"), "versions/v1/mihomo");
    assert_eq!(target(root, "previous"), "versions/v2/mihomo");
    current_is_executable(root);
    assert_eq!(rollback(root, CoreKind::Mihomo).unwrap().version, "v2");
    assert_eq!(current(root, CoreKind::Mihomo).unwrap().version, "v2");
    assert_eq!(
        fs::read(root.join("mihomo/current")).unwrap(),
        b"binary-two"
    );
    fs::set_permissions(root.join("mihomo"), fs::Permissions::from_mode(0o755)).unwrap();
}

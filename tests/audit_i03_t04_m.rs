use cm::sources::{ImportFormat, ParserError, parse_native};
use std::fs;
use std::path::PathBuf;

const PIN: &str = "v1.19.32";

fn load(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/i03_t04_m")
        .join(name);
    fs::read(path).expect("fixture")
}

fn parse(name: &str) -> Result<cm::sources::ParsedSource, ParserError> {
    parse_native(PIN, ImportFormat::MihomoJson, &load(name))
}

fn assert_hidden(error: &ParserError) {
    let text = format!("{error} {error:?}");
    assert!(!text.contains("Q01GSVhN"));
    assert!(!text.contains("CMFIXM"));
}

#[test]
fn wireguard_fixtures_match_grok_cases() {
    parse("marker-accepted.json").expect("accepted wireguard node");
    for name in [
        "key-not-base64.json",
        "key-31-bytes.json",
        "key-33-bytes.json",
        "allowed-ips-bad-cidr.json",
        "peers-overlap.json",
        "peers-over-limit.json",
    ] {
        let error = parse(name).unwrap_err();
        assert!(
            matches!(error, ParserError::InvalidNode { node_index: 0 }),
            "{name}: {error:?}"
        );
        assert_hidden(&error);
    }
    let amnezia = parse("amnezia-wg-option.json").unwrap_err();
    assert!(matches!(
        amnezia,
        ParserError::UnsupportedNodeFeature { node_index: 0 }
    ));
    assert_hidden(&amnezia);
    for name in [
        "dialer-proxy.json",
        "remote-dns-resolve.json",
        "dns.json",
    ] {
        let error = parse(name).unwrap_err();
        assert!(
            matches!(error, ParserError::RestrictedNodeOption { node_index: 0 }),
            "{name}: {error:?}"
        );
        assert_hidden(&error);
    }
}

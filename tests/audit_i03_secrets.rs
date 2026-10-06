use cm::sources::{ImportFormat, ParserError, parse_native};
use std::fs;
use std::path::PathBuf;

fn load(name: &str) -> String {
    fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/i03_t04_o")
            .join(name),
    )
    .unwrap()
}

fn visible(error: &ParserError) -> String {
    format!("{error} {error:?}")
}

#[test]
fn secret_markers_stay_out_of_errors_and_debug() {
    let accepted = [
        "vless.json",
        "vmess.json",
        "ss.json",
        "trojan.json",
        "hysteria2.json",
        "tuic.json",
        "http.json",
        "socks.json",
        "wireguard.json",
        "tls-pem.json",
    ];
    for name in accepted {
        let body = load(name);
        let parsed = parse_native("v1.19.32", ImportFormat::MihomoJson, body.as_bytes())
            .unwrap_or_else(|error| panic!("{name}: {error:?}"));
        let debug = format!("{parsed:?}");
        assert!(!debug.contains("CMFIXO"), "{name} debug leaked");
        assert!(!debug.contains("Q01GSVhP"), "{name} debug leaked key");
    }
    for (name, marker) in [
        ("tls-path.json", "/tmp/CMFIXO-tls-path.pem"),
        ("auth-field.json", "CMFIXO-auth"),
        ("header-protection.json", "Q01GSVhPLWhlYWRlci1wcm90ZWN0"),
        ("ss-plugin-password.json", "CMFIXO-plugin-password"),
        ("tuic-token.json", "CMFIXO-tuic-token"),
    ] {
        let error = parse_native(
            "v1.19.32",
            ImportFormat::MihomoJson,
            load(name).as_bytes(),
        )
        .unwrap_err();
        assert!(!visible(&error).contains(marker), "{name}");
    }
}

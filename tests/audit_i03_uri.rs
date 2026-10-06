use cm::sources::parser::parse_share_uri;
use std::fs;
use std::path::PathBuf;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/i03_t04_r/pairs")
}

fn pair(uri_name: &str, json_name: &str) {
    let uri = fs::read_to_string(dir().join(uri_name)).unwrap();
    let native = fs::read_to_string(dir().join(json_name)).unwrap();
    let parsed = parse_share_uri(uri.trim()).unwrap_or_else(|error| panic!("{uri_name}: {error:?}"));
    let expected: serde_json::Value = serde_json::from_str(&native).unwrap();
    assert_eq!(parsed, expected, "{uri_name}");
}

#[test]
fn share_uris_match_native_nodes() {
    pair("vless.uri.txt", "vless.json");
    pair("vless-upper.uri.txt", "vless.json");
    pair("vless-path.uri.txt", "vless-path.json");
    pair("vless-ipv6.uri.txt", "vless-ipv6.json");
    pair("vmess.uri.txt", "vmess.json");
    pair("vmess-nopad.uri.txt", "vmess.json");
    pair("ss-sip002.uri.txt", "ss.json");
    pair("ss-legacy.uri.txt", "ss.json");
    pair("ss-percent.uri.txt", "ss-percent.json");
    pair("trojan.uri.txt", "trojan.json");
    pair("hysteria2.uri.txt", "hysteria2.json");
    pair("hy2.uri.txt", "hysteria2.json");
    pair("tuic.uri.txt", "tuic.json");
    pair("socks5.uri.txt", "socks5.json");
    pair("http.uri.txt", "http.json");
    pair("https.uri.txt", "https.json");
}

// ---- Coordinator review 2026-10-06 ----

fn reject_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/i03_t04_r/reject")
}

/// Grok `I03.T04.r` rejection catalogue: class per cases.md, no input text in the error.
#[test]
fn grok_rejection_catalogue() {
    use cm::sources::ParserError::*;
    let cases: &[(&str, fn(&cm::sources::ParserError) -> bool)] = &[
        ("ipv6-bare.uri.txt", |e| matches!(e, InvalidNode { .. })),
        ("port-0.uri.txt", |e| matches!(e, InvalidNode { .. })),
        ("port-65536.uri.txt", |e| matches!(e, InvalidNode { .. })),
        ("port-nan.uri.txt", |e| matches!(e, InvalidNode { .. })),
        ("query-repeat.uri.txt", |e| matches!(e, DuplicateKey)),
        ("query-unknown.uri.txt", |e| matches!(e, UnsupportedNodeField { .. })),
        ("empty-fragment.uri.txt", |e| matches!(e, InvalidNode { .. })),
        ("vmess-extra.uri.txt", |e| matches!(e, UnsupportedNodeField { .. })),
        ("vmess-non-utf8.uri.txt", |e| matches!(e, InvalidUtf8)),
        ("ss-plugin.uri.txt", |e| matches!(e, UnsupportedNodeFeature { .. })),
        ("uri-over-limit.uri.txt", |e| matches!(e, UriTooLong)),
    ];
    for (file, expected) in cases {
        let line = fs::read_to_string(reject_dir().join(file)).unwrap();
        let error = parse_share_uri(line.trim_end_matches('\n')).expect_err(file);
        assert!(expected(&error), "{file}: {error:?}");
        let shown = format!("{error} {error:?}");
        assert!(!shown.contains("CMFIXR") && !shown.contains("obfs-local"), "{file}: {shown}");
    }
}

const UUID: &str = "123e4567-e89b-12d3-a456-426614174000";
const PBK: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

/// A `security` mode is mapped or refused, never downgraded to plaintext.
#[test]
fn vless_security_is_never_silently_dropped() {
    let reality = parse_share_uri(&format!(
        "vless://{UUID}@edge.example.invalid:443?type=tcp&security=reality&pbk={PBK}&sid=ab&fp=chrome&sni=www.example.com&flow=xtls-rprx-vision&encryption=none#r"
    ))
    .unwrap();
    assert_eq!(reality["tls"], true);
    assert_eq!(reality["reality-opts"]["public-key"], PBK);
    assert_eq!(reality["reality-opts"]["short-id"], "ab");
    assert_eq!(reality["client-fingerprint"], "chrome");
    assert_eq!(reality["servername"], "www.example.com");
    assert_eq!(reality["flow"], "xtls-rprx-vision");
    for security in ["xtls", "unknown"] {
        let uri = format!("vless://{UUID}@edge.example.invalid:443?security={security}#x");
        assert!(matches!(
            parse_share_uri(&uri).unwrap_err(),
            cm::sources::ParserError::UnsupportedNodeFeature { .. }
        ));
    }
    // Reality key without reality security is malformed, not ignored.
    let stray = format!("vless://{UUID}@edge.example.invalid:443?security=tls&pbk={PBK}#x");
    assert!(parse_share_uri(&stray).is_err());
}

#[test]
fn transports_and_d2_insecure_map_to_native_fields() {
    let ws = parse_share_uri(&format!(
        "vless://{UUID}@edge.example.invalid:443?type=ws&security=tls&host=cdn.example.com&path=%2Fws&sni=cdn.example.com&alpn=h2,http%2F1.1#ws"
    ))
    .unwrap();
    assert_eq!(ws["ws-opts"]["path"], "/ws");
    assert_eq!(ws["ws-opts"]["headers"]["Host"], "cdn.example.com");
    assert_eq!(ws["alpn"], serde_json::json!(["h2", "http/1.1"]));
    let grpc = parse_share_uri(&format!(
        "trojan://secret@edge.example.invalid:443?type=grpc&serviceName=svc&allowInsecure=1#g"
    ))
    .unwrap();
    assert_eq!(grpc["grpc-opts"]["grpc-service-name"], "svc");
    assert_eq!(grpc["skip-cert-verify"], true);
    let hy2 = parse_share_uri("hy2://pass@edge.example.invalid:443?insecure=1&obfs=salamander&obfs-password=o#h").unwrap();
    assert_eq!(hy2["skip-cert-verify"], true);
    // No implicit insecure: absent parameter leaves the field absent.
    let plain = parse_share_uri("trojan://secret@edge.example.invalid:443#t").unwrap();
    assert!(plain.get("skip-cert-verify").is_none());
    // Unsupported header obfuscation is refused, not dropped.
    assert!(parse_share_uri("trojan://secret@edge.example.invalid:443?headerType=http#t").is_err());
}

#[test]
fn http_and_socks_without_credentials_are_accepted() {
    let http = parse_share_uri("http://edge.example.invalid:8080#h").unwrap();
    assert!(http.get("username").is_none() && http.get("password").is_none());
    let socks = parse_share_uri("socks5://edge.example.invalid:1080#s").unwrap();
    assert!(socks.get("username").is_none());
    // User without password is ambiguous: refused.
    assert!(parse_share_uri("http://user@edge.example.invalid:8080#h").is_err());
}

#[test]
fn vmess_json_transport_fields_are_mapped() {
    use base64_shim::encode;
    let json = format!(
        r#"{{"v":"2","ps":"vm","add":"edge.example.invalid","port":"443","id":"{UUID}","aid":"0","scy":"auto","net":"ws","type":"none","host":"cdn.example.com","path":"/v","tls":"tls","sni":"cdn.example.com"}}"#
    );
    let node = parse_share_uri(&format!("vmess://{}", encode(json.as_bytes()))).unwrap();
    assert_eq!(node["network"], "ws");
    assert_eq!(node["ws-opts"]["path"], "/v");
    assert_eq!(node["ws-opts"]["headers"]["Host"], "cdn.example.com");
    assert_eq!(node["servername"], "cdn.example.com");
    assert_eq!(node["tls"], true);
}

mod base64_shim {
    pub fn encode(bytes: &[u8]) -> String {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk.iter().fold(0u32, |acc, b| (acc << 8) | u32::from(*b)) << (8 * (3 - chunk.len()));
            for i in 0..=chunk.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            }
            out.push_str(&"=".repeat(3 - chunk.len()));
        }
        out
    }
}

use cm::profiles::TlsVerification;
use cm::sources::{ImportFormat, ParserError, parse_native};

const PIN: &str = "v1.19.32";
const UUID: &str = "123e4567-e89b-12d3-a456-426614174000";
const PIN_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn parse(body: &str) -> Result<cm::sources::ParsedSource, ParserError> {
    parse_native(PIN, ImportFormat::MihomoJson, body.as_bytes())
}

fn vless(extra: &str) -> String {
    format!(
        r#"{{"proxies":[{{"name":"n","type":"vless","server":"edge.example","port":443,"uuid":"{UUID}","tls":true{extra}}}]}}"#
    )
}

#[test]
fn skip_cert_verify_without_pin_is_disabled() {
    let parsed = parse(&vless(r#","skip-cert-verify":true"#)).unwrap();
    assert_eq!(parsed.omissions().tls_verification_disabled_count, 1);
    let (_, definitions, _) = parsed.into_parts();
    assert_eq!(definitions[0].tls_verification(), TlsVerification::Disabled);
}

#[test]
fn skip_cert_verify_with_fingerprint_is_pinned() {
    let parsed = parse(&vless(&format!(
        r#","skip-cert-verify":true,"fingerprint":"{PIN_HEX}""#
    )))
    .unwrap();
    assert_eq!(parsed.omissions().tls_verification_disabled_count, 0);
    let (_, definitions, _) = parsed.into_parts();
    assert_eq!(definitions[0].tls_verification(), TlsVerification::Pinned);
}

#[test]
fn false_skip_stays_verified() {
    let parsed = parse(&vless(r#","skip-cert-verify":false,"servername":"edge.example","alpn":["h2"],"client-fingerprint":"chrome""#)).unwrap();
    let (_, definitions, _) = parsed.into_parts();
    assert_eq!(definitions[0].tls_verification(), TlsVerification::Verified);
}

#[test]
fn rejects_bad_fingerprint_unknown_utls_and_host_certs() {
    assert!(matches!(
        parse(&vless(r#","fingerprint":"chrome""#)).unwrap_err(),
        ParserError::InvalidNode { node_index: 0 }
    ));
    assert!(matches!(
        parse(&vless(r#","client-fingerprint":"not-a-browser""#)).unwrap_err(),
        ParserError::InvalidNode { node_index: 0 }
    ));
    assert!(matches!(
        parse(&vless(r#","certificate":"/etc/ssl/cert.pem""#)).unwrap_err(),
        ParserError::RestrictedNodeOption { node_index: 0 }
    ));
    assert!(matches!(
        parse(&vless(r#","ech-opts":{"enable":true}"#)).unwrap_err(),
        ParserError::UnsupportedNodeFeature { node_index: 0 }
    ));
}

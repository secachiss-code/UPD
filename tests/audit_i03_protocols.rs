use cm::sources::{ImportFormat, ParserError, parse_native};

const PIN: &str = "v1.19.32";
const UUID: &str = "123e4567-e89b-12d3-a456-426614174000";

fn parse(body: &str) -> Result<cm::sources::ParsedSource, ParserError> {
    parse_native(PIN, ImportFormat::MihomoJson, body.as_bytes())
}

fn proxy(kind: &str, extra: &str) -> String {
    format!(
        r#"{{"proxies":[{{"name":"n","type":"{kind}","server":"edge.example","port":443{extra}}}]}}"#
    )
}

#[test]
fn reality_requires_servername_and_rejects_websocket() {
    let key = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let ok = proxy(
        "vless",
        &format!(
            r#","uuid":"{UUID}","tls":true,"servername":"edge.example","client-fingerprint":"chrome","reality-opts":{{"public-key":"{key}","short-id":"ab"}}"#
        ),
    );
    parse(&ok).expect("reality on tcp");
    // Pinned core refuses REALITY without uTLS fingerprint at dial time and without TLS at
    // construction; both are refused at import (coordinator review 2026-10-06).
    for broken in [
        format!(r#","uuid":"{UUID}","tls":true,"servername":"edge.example","reality-opts":{{"public-key":"{key}"}}"#),
        format!(r#","uuid":"{UUID}","tls":true,"servername":"edge.example","client-fingerprint":"none","reality-opts":{{"public-key":"{key}"}}"#),
        format!(r#","uuid":"{UUID}","tls":false,"servername":"edge.example","client-fingerprint":"chrome","reality-opts":{{"public-key":"{key}"}}"#),
        format!(r#","uuid":"{UUID}","tls":true,"servername":"edge.example","client-fingerprint":"chrome","reality-opts":{{"public-key":"{key}="}}"#),
    ] {
        assert!(matches!(
            parse(&proxy("vless", &broken)).unwrap_err(),
            ParserError::InvalidNode { .. }
        ));
    }
    let ws = proxy(
        "vless",
        &format!(
            r#","uuid":"{UUID}","tls":true,"servername":"edge.example","network":"ws","reality-opts":{{"public-key":"{key}"}}"#
        ),
    );
    assert!(matches!(
        parse(&ws).unwrap_err(),
        ParserError::UnsupportedTransport { .. }
    ));
    let missing = proxy(
        "vless",
        &format!(r#","uuid":"{UUID}","tls":true,"reality-opts":{{"public-key":"{key}"}}"#),
    );
    assert!(matches!(
        parse(&missing).unwrap_err(),
        ParserError::InvalidNode { .. }
    ));
}

#[test]
fn vless_flow_is_tcp_tls_only() {
    let ok = proxy(
        "vless",
        &format!(r#","uuid":"{UUID}","tls":true,"flow":"xtls-rprx-vision""#),
    );
    parse(&ok).unwrap();
    let ws = proxy(
        "vless",
        &format!(r#","uuid":"{UUID}","tls":true,"network":"ws","flow":"xtls-rprx-vision""#),
    );
    assert!(parse(&ws).is_err());
}

#[test]
fn hysteria2_and_tuic_and_trojan() {
    parse(&proxy(
        "hysteria2",
        r#","password":"secret","ports":"443,20000-20010","obfs":"salamander","obfs-password":"obfs-secret""#,
    ))
    .unwrap();
    assert!(parse(&proxy("hysteria2", r#","password":"secret","ports":"70000""#)).is_err());
    assert!(parse(&proxy("hysteria2", r#","password":"secret","ports":"200-100""#)).is_err());
    assert!(parse(&proxy("hysteria2", r#","password":"secret","obfs":"salamander""#)).is_err());

    parse(&proxy(
        "tuic",
        &format!(r#","uuid":"{UUID}","password":"secret","congestion-controller":"bbr","udp-relay-mode":"quic""#),
    ))
    .unwrap();
    assert!(matches!(
        parse(&proxy("tuic", &format!(r#","uuid":"{UUID}","password":"secret","token":"v4""#))).unwrap_err(),
        ParserError::UnsupportedNodeFeature { .. }
    ));
    assert!(parse(&proxy(
        "tuic",
        &format!(r#","uuid":"{UUID}","password":"secret","congestion-controller":"yeah""#),
    ))
    .is_err());

    parse(&proxy("trojan", r#","password":"secret","network":"ws","ws-opts":{"path":"/t"}"#)).unwrap();
    parse(&proxy(
        "trojan",
        r#","password":"secret","network":"grpc","tls":true,"grpc-opts":{"grpc-service-name":"Trojan"}"#,
    ))
    .unwrap();
    assert!(parse(&proxy("trojan", r#","network":"tcp""#)).is_err());
    assert!(matches!(
        parse(&proxy("trojan", r#","password":"secret","network":"h2","tls":true"#)).unwrap_err(),
        ParserError::UnsupportedTransport { .. }
    ));
    assert!(matches!(
        parse(&proxy("trojan", r#","password":"secret","ss-opts":{"enabled":true}"#)).unwrap_err(),
        ParserError::UnsupportedNodeFeature { .. }
    ));
}

#[test]
fn shadowsocks_2022_checks_key_length_and_rejects_plugins() {
    let key = "AAAAAAAAAAAAAAAAAAAAAA==";
    parse(&proxy(
        "ss",
        &format!(r#","cipher":"2022-blake3-aes-128-gcm","password":"{key}:{key}""#),
    ))
    .unwrap();
    assert!(parse(&proxy(
        "ss",
        r#","cipher":"2022-blake3-aes-128-gcm","password":"AAAA""#,
    ))
    .is_err());
    assert!(matches!(
        parse(&proxy(
            "ss",
            r#","cipher":"aes-128-gcm","password":"secret","plugin":"obfs""#,
        ))
        .unwrap_err(),
        ParserError::UnsupportedNodeFeature { .. }
    ));
}

use cm::sources::{ImportFormat, ParserError, parse_native};

const PIN: &str = "v1.19.32";

fn parse(body: &str) -> Result<cm::sources::ParsedSource, ParserError> {
    parse_native(PIN, ImportFormat::MihomoJson, body.as_bytes())
}

fn node(extra: &str) -> String {
    format!(
        r#"{{"proxies":[{{"name":"n","type":"vless","server":"edge.example","port":443,"uuid":"123e4567-e89b-12d3-a456-426614174000"{extra}}}]}}"#
    )
}

#[test]
fn accepts_ws_http_h2_grpc_and_xhttp() {
    parse(&node(
        r#","network":"ws","ws-opts":{"path":"/v","headers":{"Host":"edge.example"},"max-early-data":2048}"#,
    ))
    .unwrap();
    parse(&node(r#","network":"http","http-opts":{"method":"GET","path":["/"],"headers":{"Host":["edge.example"]}}"#))
        .unwrap();
    parse(&node(r#","network":"h2","tls":true,"h2-opts":{"host":["edge.example"],"path":"/h2"}"#)).unwrap();
    parse(&node(
        r#","network":"grpc","tls":true,"grpc-opts":{"grpc-service-name":"GunService"}"#,
    ))
    .unwrap();
    parse(&node(r#","network":"xhttp","xhttp-opts":{"path":"/x","host":"edge.example","mode":"auto"}"#))
        .unwrap();
}

#[test]
fn rejects_bad_paths_types_and_header_injection() {
    assert!(matches!(
        parse(&node(r#","network":"ws","ws-opts":{"path":"vless","max-early-data":"2048"}"#)).unwrap_err(),
        ParserError::InvalidNode { .. }
    ));
    assert!(matches!(
        parse(&node(
            r#","network":"ws","ws-opts":{"v2ray-http-upgrade":true,"v2ray-http-upgrade-fast-open":true}"#
        ))
        .unwrap_err(),
        ParserError::InvalidNode { .. }
    ));
    assert!(matches!(
        parse(&node(r#","network":"http","http-opts":{"path":[],"headers":{"X":["a\r\nB: c"]}}"#)).unwrap_err(),
        ParserError::InvalidNode { .. }
    ));
    assert!(matches!(
        parse(&node(r#","network":"grpc","grpc-opts":{"grpc-user-agent":"x"}"#)).unwrap_err(),
        ParserError::UnsupportedNodeField { .. }
    ));
    let vmess = r#"{"proxies":[{"name":"n","type":"vmess","server":"edge.example","port":443,"uuid":"123e4567-e89b-12d3-a456-426614174000","cipher":"auto","alterId":0,"network":"xhttp"}]}"#;
    assert!(matches!(
        parse(vmess).unwrap_err(),
        ParserError::UnsupportedTransport { .. }
    ));
}

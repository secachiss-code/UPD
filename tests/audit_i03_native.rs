use cm::profiles::NodeProtocol;
use cm::sources::{
    ImportFormat, MAX_NATIVE_DEPTH, MAX_NATIVE_ENTRIES, MAX_NATIVE_NODES, ParsedSource,
    ParserError, parse_native,
};
use sha2::{Digest, Sha256};

const PIN: &str = "v1.19.32";

fn json_source(body: &str) -> Result<ParsedSource, ParserError> {
    parse_native(PIN, ImportFormat::MihomoJson, body.as_bytes())
}

#[test]
fn parses_bounded_plain_tcp_nodes_and_constrained_defaults() {
    let body = r#"{
          "mode":"rule",
          "log-level":"warning",
          "unified-delay":true,
          "tcp-concurrent":false,
          "proxies":[
            {"name":"edge-vless","type":"vless","server":"edge.example","port":443,"uuid":"123e4567-e89b-12d3-a456-426614174000","network":"tcp","udp":true},
            {"name":"edge-vmess","type":"vmess","server":"2001:db8::1","port":8443,"uuid":"123e4567-e89b-12d3-a456-426614174001","cipher":"auto","alterId":0},
            {"name":"edge-ss","type":"ss","server":"ss.example","port":8388,"cipher":"aes-128-gcm","password":"private-pass"}
          ]
        }"#;
    let parsed = json_source(body).expect("synthetic source fits the documented parser subset");

    assert_eq!(parsed.format(), ImportFormat::MihomoJson);
    assert_eq!(parsed.node_count(), 3);
    let expected_digest = Sha256::digest(body.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(parsed.source_body_sha256(), expected_digest);
    assert_eq!(
        parsed
            .defaults()
            .get("log-level")
            .and_then(|value| value.as_str()),
        Some("warning")
    );
    let debug = format!("{parsed:?}");
    assert!(!debug.contains("private-pass"));
    assert!(!debug.contains("123e4567-e89b-12d3-a456-426614174000"));

    let (_, definitions, _) = parsed.into_parts();
    assert_eq!(definitions[0].protocol(), NodeProtocol::Vless);
    assert_eq!(definitions[1].protocol(), NodeProtocol::Vmess);
}

#[test]
fn rejects_duplicate_json_and_nested_yaml_keys() {
    let duplicate_json = r#"{"proxies":[{"name":"n","type":"ss","server":"edge.example","port":443,"cipher":"aes-128-gcm","password":"one","password":"two"}]}"#;
    assert_eq!(
        json_source(duplicate_json).unwrap_err(),
        ParserError::DuplicateKey
    );

    let duplicate_yaml = b"proxies:\n  - name: edge\n    type: ss\n    server: edge.example\n    port: 443\n    cipher: aes-128-gcm\n    password: one\n    password: two\n";
    assert_eq!(
        parse_native(PIN, ImportFormat::MihomoYaml, duplicate_yaml).unwrap_err(),
        ParserError::DuplicateKey
    );
}

#[test]
fn quoted_yaml_secret_markers_remain_plain_data_and_debug_stays_redacted() {
    let yaml = b"proxies:\n  - name: edge\n    type: ss\n    server: edge.example\n    port: 8388\n    cipher: aes-128-gcm\n    password: 'a &anchor *alias !tag <<'\n";
    let parsed = parse_native(PIN, ImportFormat::MihomoYaml, yaml).unwrap();
    assert!(!format!("{parsed:?}").contains("a &anchor"));
    let (_, definitions, _) = parsed.into_parts();
    assert_eq!(
        definitions[0].full_definition()["password"].as_str(),
        Some("a &anchor *alias !tag <<")
    );
}

#[test]
fn rejects_yaml_aliases_tags_merges_and_multiple_documents_before_loading() {
    for yaml in [
        b"proxies: &private\n  - name: edge\n".as_slice(),
        b"proxies: *private\n".as_slice(),
        b"proxies: !private []\n".as_slice(),
        b"proxies:\n  - <<: {name: edge}\n".as_slice(),
        b"proxies: []\n---\nproxies: []\n".as_slice(),
    ] {
        assert!(parse_native(PIN, ImportFormat::MihomoYaml, yaml).is_err());
    }
}

#[test]
fn rejects_host_control_fields_sections_unknown_fields_and_unsupported_branches() {
    let restricted = r#"{"proxies":[{"name":"n","type":"ss","server":"edge.example","port":443,"cipher":"aes-128-gcm","password":"p","interface-name":"eth0"}]}"#;
    assert_eq!(
        json_source(restricted).unwrap_err(),
        ParserError::RestrictedNodeOption { node_index: 0 }
    );

    let provider = r#"{"proxies":[{"name":"n","type":"ss","server":"edge.example","port":443,"cipher":"aes-128-gcm","password":"p"}],"proxy-providers":{"remote":{"type":"http"}}}"#;
    let parsed = json_source(provider).expect("http providers are omitted, not fetched");
    assert!(parsed
        .omissions()
        .section_names
        .iter()
        .any(|name| name == "proxy-providers"));

    let unknown = r#"{"proxies":[{"name":"n","type":"ss","server":"edge.example","port":443,"cipher":"aes-128-gcm","password":"p","new-core-option":true}]}"#;
    assert_eq!(
        json_source(unknown).unwrap_err(),
        ParserError::UnsupportedNodeField { node_index: 0 }
    );

    let tls = r#"{"proxies":[{"name":"n","type":"vless","server":"edge.example","port":443,"uuid":"123e4567-e89b-12d3-a456-426614174000","tls":true}]}"#;
    let parsed = json_source(tls).expect("tls without skip-cert-verify stays verified");
    let (_, definitions, _) = parsed.into_parts();
    assert_eq!(
        definitions[0].tls_verification(),
        cm::profiles::TlsVerification::Verified
    );

    let websocket = r#"{"proxies":[{"name":"n","type":"vless","server":"edge.example","port":443,"uuid":"123e4567-e89b-12d3-a456-426614174000","network":"ws","ws-opts":{"path":"no-slash"}}]}"#;
    assert_eq!(
        json_source(websocket).unwrap_err(),
        ParserError::InvalidNode { node_index: 0 }
    );

    let native_group = r#"{"proxies":[{"name":"n","type":"ss","server":"edge.example","port":443,"cipher":"aes-128-gcm","password":"p"}],"proxy-groups":[],"rules":[]}"#;
    let parsed = json_source(native_group).expect("groups and rules are recorded omissions");
    assert_eq!(
        parsed.omissions().section_names,
        ["proxy-groups".to_string(), "rules".to_string()]
    );

    let ss2022 = r#"{"proxies":[{"name":"n","type":"ss","server":"edge.example","port":443,"cipher":"2022-blake3-aes-128-gcm","password":"private-psk"}]}"#;
    let error = json_source(ss2022).unwrap_err();
    assert_eq!(error, ParserError::InvalidNode { node_index: 0 });
    assert!(!format!("{error:?} {error}").contains("private-psk"));
}

#[test]
fn header_names_are_not_misclassified_as_node_controls() {
    let source = r#"{"proxies":[{"name":"http-node","type":"http","server":"proxy.example","port":8080,"headers":{"dns":"kept","plugin":"also-kept","X-Request":"safe"}}]}"#;
    let parsed = json_source(source).unwrap();
    let (_, definitions, _) = parsed.into_parts();
    assert_eq!(definitions[0].full_definition()["headers"]["dns"], "kept");
    assert_eq!(
        definitions[0].full_definition()["headers"]["plugin"],
        "also-kept"
    );

    let duplicate_header = r#"{"proxies":[{"name":"http-node","type":"http","server":"proxy.example","port":8080,"headers":{"Host":"one","host":"two"}}]}"#;
    assert_eq!(
        json_source(duplicate_header).unwrap_err(),
        ParserError::InvalidNode { node_index: 0 }
    );

    let header_control = r#"{"proxies":[{"name":"http-node","type":"http","server":"proxy.example","port":8080,"headers":{"X-Test":"bad\u0007value"}}]}"#;
    assert_eq!(
        json_source(header_control).unwrap_err(),
        ParserError::InvalidNode { node_index: 0 }
    );

    let socks = r#"{"proxies":[{"name":"socks","type":"socks5","server":"proxy.example","port":1080,"username":"user","password":"private-pass"}]}"#;
    let parsed = json_source(socks).unwrap();
    let (_, definitions, _) = parsed.into_parts();
    assert_eq!(definitions[0].protocol(), NodeProtocol::Socks5);
    assert!(!format!("{:?}", definitions[0]).contains("private-pass"));

    let partial_socks_auth = r#"{"proxies":[{"name":"socks","type":"socks5","server":"proxy.example","port":1080,"username":"user"}]}"#;
    assert_eq!(
        json_source(partial_socks_auth).unwrap_err(),
        ParserError::InvalidNode { node_index: 0 }
    );
}

#[test]
fn validates_limits_defaults_hosts_ports_and_core_pin() {
    assert_eq!(
        parse_native("1.19.31", ImportFormat::MihomoJson, b"{}").unwrap_err(),
        ParserError::UnsupportedCoreVersion
    );
    assert_eq!(
        parse_native("v1.19.32", ImportFormat::UriList, b"vless://private").unwrap_err(),
        ParserError::UnsupportedFormat
    );

    let invalid_level = r#"{"log-level":"warn","proxies":[{"name":"n","type":"ss","server":"edge.example","port":443,"cipher":"aes-128-gcm","password":"p"}]}"#;
    assert_eq!(
        json_source(invalid_level).unwrap_err(),
        ParserError::InvalidDefault
    );

    let removed_fingerprint = r#"{"global-client-fingerprint":"chrome","proxies":[{"name":"n","type":"ss","server":"edge.example","port":443,"cipher":"aes-128-gcm","password":"p"}]}"#;
    assert_eq!(
        json_source(removed_fingerprint).unwrap_err(),
        ParserError::UnsupportedNativeField
    );

    let url_host = r#"{"proxies":[{"name":"n","type":"ss","server":"https://edge.example","port":443,"cipher":"aes-128-gcm","password":"p"}]}"#;
    assert_eq!(
        json_source(url_host).unwrap_err(),
        ParserError::InvalidNode { node_index: 0 }
    );

    let bad_port = r#"{"proxies":[{"name":"n","type":"ss","server":"edge.example","port":65536,"cipher":"aes-128-gcm","password":"p"}]}"#;
    assert_eq!(
        json_source(bad_port).unwrap_err(),
        ParserError::InvalidNode { node_index: 0 }
    );

    let too_large = vec![b' '; 8 * 1024 * 1024 + 1];
    assert_eq!(
        parse_native(PIN, ImportFormat::MihomoJson, &too_large).unwrap_err(),
        ParserError::BodyTooLarge
    );
}

#[test]
fn rejects_node_name_collisions_and_the_node_limit() {
    let repeated = r#"{"proxies":[{"name":"same","type":"ss","server":"edge.example","port":443,"cipher":"aes-128-gcm","password":"p"},{"name":"same","type":"ss","server":"edge.example","port":443,"cipher":"aes-128-gcm","password":"q"}]}"#;
    assert_eq!(
        json_source(repeated).unwrap_err(),
        ParserError::DuplicateNodeName { node_index: 1 }
    );

    let nodes = (0..=MAX_NATIVE_NODES)
        .map(|index| {
            format!(
                r#"{{"name":"n{index}","type":"ss","server":"edge.example","port":443,"cipher":"aes-128-gcm","password":"p"}}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let oversized = format!(r#"{{"proxies":[{nodes}]}}"#);
    assert_eq!(
        parse_native(PIN, ImportFormat::MihomoJson, oversized.as_bytes()).unwrap_err(),
        ParserError::TooManyNodes
    );
}

#[test]
fn rejects_excessive_nesting_and_keeps_errors_secret_free() {
    let deeply_nested = format!(
        r#"{{"proxies":[],"unknown":{}null{}}}"#,
        "[".repeat(MAX_NATIVE_DEPTH),
        "]".repeat(MAX_NATIVE_DEPTH)
    );
    let error = json_source(&deeply_nested).unwrap_err();
    assert_eq!(error, ParserError::TooDeep);
    assert!(!error.to_string().contains("unknown"));

    let values = std::iter::repeat("0")
        .take(MAX_NATIVE_ENTRIES + 1)
        .collect::<Vec<_>>()
        .join(",");
    let too_many_values = format!(r#"{{"proxies":[],"unknown":[{values}]}}"#);
    assert_eq!(
        json_source(&too_many_values).unwrap_err(),
        ParserError::TooManyEntries
    );
}

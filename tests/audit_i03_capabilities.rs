//! Static capability policy regressions; these do not parse sources or run mihomo.

use cm::profiles::NodeProtocol;
use cm::sources::*;

#[test]
fn core_pin_and_import_formats_are_explicit() {
    assert_eq!(PINNED_CORE_VERSION, "1.19.32");
    assert_eq!(
        PINNED_CORE_COMMIT,
        "88dcbf7f1614a67c3b36b848ee3592dfa92ada36"
    );
    assert_eq!(
        supported_import_formats(),
        &[
            ImportFormat::UriList,
            ImportFormat::Base64UriList,
            ImportFormat::MihomoYaml,
            ImportFormat::MihomoJson,
        ]
    );
}

#[test]
fn protocol_transport_matrix_accepts_only_the_pinned_subset() {
    let accepted = [
        (NodeProtocol::Vless, Transport::Tcp),
        (NodeProtocol::Vless, Transport::Ws),
        (NodeProtocol::Vless, Transport::Http),
        (NodeProtocol::Vless, Transport::H2),
        (NodeProtocol::Vless, Transport::Grpc),
        (NodeProtocol::Vless, Transport::Xhttp),
        (NodeProtocol::Vmess, Transport::Tcp),
        (NodeProtocol::Vmess, Transport::Ws),
        (NodeProtocol::Vmess, Transport::Http),
        (NodeProtocol::Vmess, Transport::H2),
        (NodeProtocol::Vmess, Transport::Grpc),
        (NodeProtocol::Trojan, Transport::Tcp),
        (NodeProtocol::Trojan, Transport::Ws),
        (NodeProtocol::Trojan, Transport::Grpc),
        (NodeProtocol::Shadowsocks, Transport::Tcp),
        (NodeProtocol::Socks5, Transport::Tcp),
        (NodeProtocol::Http, Transport::Tcp),
        (NodeProtocol::Hysteria2, Transport::Quic),
        (NodeProtocol::Tuic, Transport::Quic),
        (NodeProtocol::WireGuard, Transport::WireGuard),
    ];
    for (protocol, transport) in accepted {
        assert_eq!(validate("v1.19.32", protocol, transport), Ok(()));
    }

    let rejected = [
        (NodeProtocol::Vmess, Transport::Xhttp),
        (NodeProtocol::Trojan, Transport::H2),
        (NodeProtocol::Shadowsocks, Transport::Quic),
        (NodeProtocol::Socks5, Transport::Ws),
        (NodeProtocol::Hysteria2, Transport::Tcp),
        (NodeProtocol::WireGuard, Transport::Tcp),
    ];
    for (protocol, transport) in rejected {
        assert_eq!(
            validate(PINNED_CORE_VERSION, protocol, transport),
            Err(CapabilityError::UnsupportedTransport)
        );
    }
    assert_eq!(
        validate(PINNED_CORE_VERSION, NodeProtocol::Other, Transport::Tcp),
        Err(CapabilityError::UnsupportedProtocol)
    );
}

#[test]
fn wrong_core_versions_and_uri_schemes_fail_with_safe_unit_errors() {
    for version in ["1.19.31", "1.19.33", "v1.19.32-1", "mihomo/1.19.32"] {
        let error = validate(version, NodeProtocol::Vless, Transport::Tcp).unwrap_err();
        assert_eq!(error, CapabilityError::UnsupportedCoreVersion);
        assert!(!error.to_string().contains(version));
    }
    for (scheme, expected) in [
        ("vless", NodeProtocol::Vless),
        ("VMESS", NodeProtocol::Vmess),
        ("ss", NodeProtocol::Shadowsocks),
        ("trojan", NodeProtocol::Trojan),
        ("socks", NodeProtocol::Socks5),
        ("socks5", NodeProtocol::Socks5),
        ("http", NodeProtocol::Http),
        ("https", NodeProtocol::Http),
        ("hysteria2", NodeProtocol::Hysteria2),
        ("hy2", NodeProtocol::Hysteria2),
        ("tuic", NodeProtocol::Tuic),
    ] {
        assert_eq!(protocol_for_uri_scheme(scheme), Ok(expected));
    }
    for scheme in ["xray", "sing-box", "wireguard", "unknown-private-scheme"] {
        let error = protocol_for_uri_scheme(scheme).unwrap_err();
        assert_eq!(error, CapabilityError::UnsupportedUriScheme);
        assert!(!error.to_string().contains(scheme));
    }
}

#[test]
fn native_config_policy_restricts_host_controls_and_rejects_unknown_or_advanced_fields() {
    for field in ["proxies", "proxy-groups", "rules", "sub-rules"] {
        assert_ne!(
            classify_native_config_field(field).unwrap(),
            NativeFieldDisposition::RestrictedNative
        );
    }
    for field in [
        "listeners",
        "tun",
        "port",
        "mixed-port",
        "external-controller",
        "route",
        "firewall",
        "interface-name",
        "routing-mark",
        "dns",
    ] {
        assert_eq!(
            classify_native_config_field(field),
            Ok(NativeFieldDisposition::RestrictedNative)
        );
    }
    for field in [
        "ech-opts",
        "shadow-tls-opts",
        "restls-opts",
        "jls-opts",
        "plugin",
    ] {
        assert_eq!(
            classify_node_option(field),
            Err(CapabilityError::UnsupportedFeature)
        );
    }
    assert_eq!(
        classify_native_config_field("unknown-private-field"),
        Err(CapabilityError::UnsupportedField)
    );
    assert_eq!(
        classify_native_config_field("proxy-providers"),
        Ok(NativeFieldDisposition::ProviderDeclarations)
    );
    assert_eq!(
        classify_provider_mode("file"),
        Ok(ProviderMode::File),
        "file provider paths and content still need validation"
    );
    assert_eq!(classify_provider_mode("inline"), Ok(ProviderMode::Inline));
    assert_eq!(
        classify_node_option("reality-opts"),
        Ok(NodeOptionDisposition::PreservedNodeOption)
    );
    assert_eq!(
        classify_native_config_field("reality-opts"),
        Err(CapabilityError::UnsupportedField)
    );
    for field in [
        "dialer-proxy",
        "ip-stack",
        "remote-dns-resolve",
        "dns",
        "interface-name",
        "routing-mark",
    ] {
        assert_eq!(
            classify_node_option(field),
            Ok(NodeOptionDisposition::RestrictedNative)
        );
    }
    assert_eq!(
        classify_provider_mode("http"),
        Err(CapabilityError::UnsupportedProviderMode)
    );
}

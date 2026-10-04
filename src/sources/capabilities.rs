//! Static, deliberately conservative import capabilities for the pinned mihomo core.
//!
//! This module only describes policy. It performs no parsing, networking, or core execution.

use crate::profiles::NodeProtocol;
use std::fmt;

/// The exact upstream core contract reviewed for this capability subset.
pub const PINNED_CORE_VERSION: &str = "1.19.32";
/// Full upstream commit for `PINNED_CORE_VERSION`.
pub const PINNED_CORE_COMMIT: &str = "88dcbf7f1614a67c3b36b848ee3592dfa92ada36";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum ImportFormat {
    UriList,
    Base64UriList,
    MihomoYaml,
    MihomoJson,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Transport {
    Tcp,
    Ws,
    Http,
    H2,
    Grpc,
    Xhttp,
    Quic,
    WireGuard,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityError {
    UnsupportedCoreVersion,
    UnsupportedProtocol,
    UnsupportedTransport,
    UnsupportedUriScheme,
    UnsupportedFormat,
    UnsupportedProviderMode,
    UnsupportedFeature,
    UnsupportedField,
}

impl fmt::Display for CapabilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnsupportedCoreVersion => "unsupported core version",
            Self::UnsupportedProtocol => "unsupported protocol",
            Self::UnsupportedTransport => "unsupported protocol transport",
            Self::UnsupportedUriScheme => "unsupported URI scheme",
            Self::UnsupportedFormat => "unsupported import format",
            Self::UnsupportedProviderMode => "unsupported provider mode",
            Self::UnsupportedFeature => "known feature is outside the supported subset",
            Self::UnsupportedField => "unknown native configuration field",
        })
    }
}

impl std::error::Error for CapabilityError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UriSchemeCapability {
    pub scheme: &'static str,
    pub protocol: NodeProtocol,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeFieldDisposition {
    ProxyDefinitions,
    ProxyGroups,
    Rules,
    SubRules,
    ProviderDeclarations,
    ConstrainedDefault,
    RestrictedNative,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeOptionDisposition {
    PreservedNodeOption,
    RestrictedNative,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderMode {
    File,
    Inline,
}

const IMPORT_FORMATS: &[ImportFormat] = &[
    ImportFormat::UriList,
    ImportFormat::Base64UriList,
    ImportFormat::MihomoYaml,
    ImportFormat::MihomoJson,
];

const URI_SCHEMES: &[UriSchemeCapability] = &[
    UriSchemeCapability {
        scheme: "vless",
        protocol: NodeProtocol::Vless,
    },
    UriSchemeCapability {
        scheme: "vmess",
        protocol: NodeProtocol::Vmess,
    },
    UriSchemeCapability {
        scheme: "ss",
        protocol: NodeProtocol::Shadowsocks,
    },
    UriSchemeCapability {
        scheme: "trojan",
        protocol: NodeProtocol::Trojan,
    },
    UriSchemeCapability {
        scheme: "socks5",
        protocol: NodeProtocol::Socks5,
    },
    UriSchemeCapability {
        scheme: "socks",
        protocol: NodeProtocol::Socks5,
    },
    UriSchemeCapability {
        scheme: "http",
        protocol: NodeProtocol::Http,
    },
    UriSchemeCapability {
        scheme: "https",
        protocol: NodeProtocol::Http,
    },
    UriSchemeCapability {
        scheme: "hysteria2",
        protocol: NodeProtocol::Hysteria2,
    },
    UriSchemeCapability {
        scheme: "hy2",
        protocol: NodeProtocol::Hysteria2,
    },
    UriSchemeCapability {
        scheme: "tuic",
        protocol: NodeProtocol::Tuic,
    },
];

const TCP: &[Transport] = &[Transport::Tcp];
const VLESS: &[Transport] = &[
    Transport::Tcp,
    Transport::Ws,
    Transport::Http,
    Transport::H2,
    Transport::Grpc,
    Transport::Xhttp,
];
const VMESS: &[Transport] = &[
    Transport::Tcp,
    Transport::Ws,
    Transport::Http,
    Transport::H2,
    Transport::Grpc,
];
const TROJAN: &[Transport] = &[Transport::Tcp, Transport::Ws, Transport::Grpc];
const QUIC: &[Transport] = &[Transport::Quic];
const WIREGUARD: &[Transport] = &[Transport::WireGuard];

/// Accepted source representation formats. Xray and sing-box have separate adapters later.
pub const fn supported_import_formats() -> &'static [ImportFormat] {
    IMPORT_FORMATS
}

/// URI scheme to protocol mapping for the explicit supported subset.
pub const fn supported_uri_schemes() -> &'static [UriSchemeCapability] {
    URI_SCHEMES
}

pub fn protocol_for_uri_scheme(scheme: &str) -> Result<NodeProtocol, CapabilityError> {
    URI_SCHEMES
        .iter()
        .find(|entry| entry.scheme.eq_ignore_ascii_case(scheme))
        .map(|entry| entry.protocol)
        .ok_or(CapabilityError::UnsupportedUriScheme)
}

/// Return the permitted CM transport subset for a model protocol.
pub fn supported_transports(
    protocol: NodeProtocol,
) -> Result<&'static [Transport], CapabilityError> {
    match protocol {
        NodeProtocol::Vless => Ok(VLESS),
        NodeProtocol::Vmess => Ok(VMESS),
        NodeProtocol::Shadowsocks => Ok(TCP),
        NodeProtocol::Trojan => Ok(TROJAN),
        NodeProtocol::Socks5 | NodeProtocol::Http => Ok(TCP),
        NodeProtocol::Hysteria2 | NodeProtocol::Tuic => Ok(QUIC),
        NodeProtocol::WireGuard => Ok(WIREGUARD),
        NodeProtocol::Other => Err(CapabilityError::UnsupportedProtocol),
    }
}

/// Validate a transport against the reviewed core pin and conservative CM subset.
/// A single lowercase `v` prefix is optional; all other version spellings are exact.
pub fn validate(
    core_version: &str,
    protocol: NodeProtocol,
    transport: Transport,
) -> Result<(), CapabilityError> {
    let normalized = core_version.strip_prefix('v').unwrap_or(core_version);
    if normalized != PINNED_CORE_VERSION {
        return Err(CapabilityError::UnsupportedCoreVersion);
    }
    if supported_transports(protocol)?.contains(&transport) {
        Ok(())
    } else {
        Err(CapabilityError::UnsupportedTransport)
    }
}

/// Classify top-level native config keys without echoing unchecked input in diagnostics.
/// Values and nested objects still require explicit validation by a later parser stage.
pub fn classify_native_config_field(
    field: &str,
) -> Result<NativeFieldDisposition, CapabilityError> {
    let disposition = match field {
        "proxies" => NativeFieldDisposition::ProxyDefinitions,
        "proxy-groups" => NativeFieldDisposition::ProxyGroups,
        "rules" => NativeFieldDisposition::Rules,
        "sub-rules" => NativeFieldDisposition::SubRules,
        "proxy-providers" | "rule-providers" => NativeFieldDisposition::ProviderDeclarations,
        "mode" | "log-level" | "unified-delay" | "tcp-concurrent" => {
            NativeFieldDisposition::ConstrainedDefault
        }
        // Kept in RawConfig for compatibility, but parseGeneral explicitly ignores it.
        "global-client-fingerprint" => return Err(CapabilityError::UnsupportedFeature),
        "listeners"
        | "tun"
        | "port"
        | "socks-port"
        | "redir-port"
        | "tproxy-port"
        | "mixed-port"
        | "external-controller"
        | "external-controller-pipe"
        | "external-controller-unix"
        | "external-controller-cors"
        | "external-controller-tls"
        | "secret"
        | "authentication"
        | "allow-lan"
        | "bind-address"
        | "lan-allowed-ips"
        | "lan-disallowed-ips"
        | "routing-mark"
        | "interface-name"
        | "route"
        | "routes"
        | "routing"
        | "firewall"
        | "iptables"
        | "ebpf"
        | "dns"
        | "hosts"
        | "sniffer" => NativeFieldDisposition::RestrictedNative,
        _ => return Err(CapabilityError::UnsupportedField),
    };
    Ok(disposition)
}

/// Identify per-node controls that must not gain host networking ownership.
/// Other node fields, including REALITY, are left to the later protocol parser.
pub fn classify_node_option(field: &str) -> Result<NodeOptionDisposition, CapabilityError> {
    match field {
        "interface-name" | "routing-mark" | "dialer-proxy" | "ip-stack" | "remote-dns-resolve"
        | "dns" => Ok(NodeOptionDisposition::RestrictedNative),
        "reality-opts" => Ok(NodeOptionDisposition::PreservedNodeOption),
        "ech-opts" | "shadow-tls-opts" | "restls-opts" | "jls-opts" | "tlsmirror-opts"
        | "mekya-opts" | "mkcp-opts" | "plugin" | "plugin-opts" => {
            return Err(CapabilityError::UnsupportedFeature);
        }
        _ => Err(CapabilityError::UnsupportedField),
    }
}

/// Local file and inline declarations are capability-approved at this stage.
/// Paths, bounds, and content remain the responsibility of the later import implementation.
pub fn classify_provider_mode(mode: &str) -> Result<ProviderMode, CapabilityError> {
    if mode.eq_ignore_ascii_case("file") {
        Ok(ProviderMode::File)
    } else if mode.eq_ignore_ascii_case("inline") {
        Ok(ProviderMode::Inline)
    } else {
        Err(CapabilityError::UnsupportedProviderMode)
    }
}

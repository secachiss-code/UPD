mod http;
mod hy2;
mod socks;
mod ss;
mod trojan;
mod tuic;
mod vless;
mod vmess;
mod wg;

use super::ParserError;
use crate::profiles::NodeProtocol;
use crate::sources::capabilities::Transport;
use serde_json::Map;
use serde_json::Value;

pub(crate) fn protocol_and_transport(
    type_name: &str,
    object: &Map<String, Value>,
    node_index: usize,
) -> Result<(NodeProtocol, Transport), ParserError> {
    match type_name {
        "vless" => vless::protocol_and_transport(object, node_index),
        "vmess" => vmess::protocol_and_transport(object, node_index),
        "ss" => Ok(ss::protocol_and_transport()),
        "http" => Ok(http::protocol_and_transport()),
        "socks5" => Ok(socks::protocol_and_transport()),
        "trojan" => trojan::protocol_and_transport(object, node_index),
        "hysteria2" | "hy2" => Ok(hy2::protocol_and_transport()),
        "tuic" => Ok(tuic::protocol_and_transport()),
        "wireguard" => Ok(wg::protocol_and_transport()),
        _ => Err(ParserError::UnsupportedProtocol { node_index }),
    }
}

pub(crate) fn supported_fields(protocol: NodeProtocol) -> &'static [&'static str] {
    match protocol {
        NodeProtocol::Vless => vless::SUPPORTED_FIELDS,
        NodeProtocol::Vmess => vmess::SUPPORTED_FIELDS,
        NodeProtocol::Shadowsocks => ss::SUPPORTED_FIELDS,
        NodeProtocol::Http => http::SUPPORTED_FIELDS,
        NodeProtocol::Socks5 => socks::SUPPORTED_FIELDS,
        NodeProtocol::WireGuard => wg::SUPPORTED_FIELDS,
        NodeProtocol::Trojan => trojan::SUPPORTED_FIELDS,
        NodeProtocol::Hysteria2 => hy2::SUPPORTED_FIELDS,
        NodeProtocol::Tuic => tuic::SUPPORTED_FIELDS,
        _ => &[],
    }
}

pub(crate) fn validate_protocol(
    protocol: NodeProtocol,
    object: &Map<String, Value>,
    node_index: usize,
) -> Result<(), ParserError> {
    match protocol {
        NodeProtocol::Vless => vless::resolve(object, node_index),
        NodeProtocol::Vmess => vmess::validate(object, node_index),
        NodeProtocol::Shadowsocks => ss::validate(object, node_index),
        NodeProtocol::Http => http::validate(object, node_index),
        NodeProtocol::Socks5 => socks::validate(object, node_index),
        NodeProtocol::WireGuard => wg::validate(object, node_index),
        NodeProtocol::Trojan => trojan::validate(object, node_index),
        NodeProtocol::Hysteria2 => hy2::validate(object, node_index),
        NodeProtocol::Tuic => tuic::validate(object, node_index),
        _ => Err(ParserError::UnsupportedProtocol { node_index }),
    }
}

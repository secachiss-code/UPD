use super::super::ParserError;
use super::super::common::validate_uuid;
use crate::profiles::NodeProtocol;
use crate::sources::capabilities::Transport;
use serde_json::Map;
use serde_json::Value;

pub(crate) const SUPPORTED_FIELDS: &[&str] = &[
    "name",
    "type",
    "server",
    "port",
    "uuid",
    "network",
    "udp",
    "tls",
    "servername",
    "sni",
    "alpn",
    "client-fingerprint",
    "fingerprint",
    "skip-cert-verify",
    "name-cert-verify",
    "certificate",
    "private-key",
    "ws-opts",
    "http-opts",
    "h2-opts",
    "grpc-opts",
    "xhttp-opts",
    "reality-opts",
    "flow",
    "packet-encoding",
];

pub(crate) fn resolve(object: &Map<String, Value>, node_index: usize) -> Result<(), ParserError> {
    validate_uuid(object, node_index)?;
    if let Some(flow) = object.get("flow") {
        let Some(flow) = flow.as_str() else {
            return Err(ParserError::InvalidNode { node_index });
        };
        let vision = flow.len() >= 16 && &flow.as_bytes()[..16] == b"xtls-rprx-vision";
        if !vision {
            return Err(ParserError::UnsupportedNodeFeature { node_index });
        }
        let tcp = object
            .get("network")
            .and_then(Value::as_str)
            .is_none_or(|network| network == "tcp");
        let tls = object.get("tls").and_then(Value::as_bool) == Some(true);
        let reality = object.contains_key("reality-opts");
        if !tcp || !(tls || reality) {
            return Err(ParserError::InvalidNode { node_index });
        }
    }
    if let Some(encoding) = object.get("packet-encoding") {
        let Some(encoding) = encoding.as_str() else {
            return Err(ParserError::InvalidNode { node_index });
        };
        if !matches!(encoding, "packetaddr" | "xudp" | "packet") {
            return Err(ParserError::InvalidNode { node_index });
        }
    }
    if object.contains_key("reality-opts") {
        super::super::tls::validate_reality(object, node_index)?;
    }
    Ok(())
}

pub(crate) fn protocol_and_transport(
    object: &Map<String, Value>,
    node_index: usize,
) -> Result<(NodeProtocol, Transport), ParserError> {
    let transport = super::super::transport::select_network(object, node_index, true)?;
    Ok((NodeProtocol::Vless, transport))
}

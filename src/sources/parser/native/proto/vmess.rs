use super::super::ParserError;
use super::super::common::{required_string, validate_uuid};
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
    "cipher",
    "alterId",
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
    "reality-opts",
    "packet-encoding",
    "packet-addr",
    "global-padding",
    "authenticated-length",
];

pub(crate) fn protocol_and_transport(
    object: &Map<String, Value>,
    node_index: usize,
) -> Result<(NodeProtocol, Transport), ParserError> {
    let transport = super::super::transport::select_network(object, node_index, false)?;
    Ok((NodeProtocol::Vmess, transport))
}

pub(crate) fn validate(object: &Map<String, Value>, node_index: usize) -> Result<(), ParserError> {
    validate_uuid(object, node_index)?;
    let cipher = required_string(object, "cipher", node_index, 64)?;
    if !matches!(
        cipher,
        "auto" | "none" | "zero" | "aes-128-gcm" | "aes-128-cfb" | "chacha20-poly1305"
    ) {
        return Err(ParserError::InvalidNode { node_index });
    }
    if let Some(alter_id) = object.get("alterId")
        && alter_id.as_u64().is_none()
    {
        return Err(ParserError::InvalidNode { node_index });
    }
    for field in ["global-padding", "authenticated-length", "packet-addr"] {
        if object.contains_key(field) && object.get(field).and_then(Value::as_bool).is_none() {
            return Err(ParserError::InvalidNode { node_index });
        }
    }
    if let Some(encoding) = object.get("packet-encoding") {
        let Some(encoding) = encoding.as_str() else {
            return Err(ParserError::InvalidNode { node_index });
        };
        if !matches!(encoding, "packetaddr" | "packet" | "xudp" | "") {
            return Err(ParserError::InvalidNode { node_index });
        }
    }
    if object.contains_key("reality-opts") {
        super::super::tls::validate_reality(object, node_index)?;
    }
    Ok(())
}

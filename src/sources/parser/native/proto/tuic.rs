use super::super::ParserError;
use super::super::common::{MAX_SECRET_BYTES, has_control, required_string, validate_uuid};
use crate::profiles::NodeProtocol;
use crate::sources::capabilities::Transport;
use serde_json::{Map, Value};

pub(crate) const SUPPORTED_FIELDS: &[&str] = &[
    "name",
    "type",
    "server",
    "port",
    "uuid",
    "password",
    "token",
    "congestion-controller",
    "udp-relay-mode",
    "reduce-rtt",
    "alpn",
    "sni",
    "disable-sni",
    "request-timeout",
    "skip-cert-verify",
    "udp",
];

pub(crate) fn protocol_and_transport() -> (NodeProtocol, Transport) {
    (NodeProtocol::Tuic, Transport::Quic)
}

pub(crate) fn validate(object: &Map<String, Value>, node_index: usize) -> Result<(), ParserError> {
    if object.contains_key("token") {
        return Err(ParserError::UnsupportedNodeFeature { node_index });
    }
    validate_uuid(object, node_index)?;
    let password = required_string(object, "password", node_index, MAX_SECRET_BYTES)?;
    if password.is_empty() || has_control(password) {
        return Err(ParserError::InvalidNode { node_index });
    }
    if let Some(controller) = object.get("congestion-controller") {
        let Some(controller) = controller.as_str() else {
            return Err(ParserError::InvalidNode { node_index });
        };
        if !matches!(controller, "cubic" | "new_reno" | "bbr") {
            return Err(ParserError::InvalidNode { node_index });
        }
    }
    if let Some(mode) = object.get("udp-relay-mode") {
        let Some(mode) = mode.as_str() else {
            return Err(ParserError::InvalidNode { node_index });
        };
        if !matches!(mode, "native" | "quic") {
            return Err(ParserError::InvalidNode { node_index });
        }
    }
    for field in ["reduce-rtt", "disable-sni"] {
        if object.contains_key(field) && object.get(field).and_then(Value::as_bool).is_none() {
            return Err(ParserError::InvalidNode { node_index });
        }
    }
    if let Some(timeout) = object.get("request-timeout")
        && timeout.as_u64().is_none()
    {
        return Err(ParserError::InvalidNode { node_index });
    }
    Ok(())
}

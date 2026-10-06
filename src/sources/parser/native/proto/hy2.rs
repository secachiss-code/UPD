use super::super::ParserError;
use super::super::common::{MAX_SECRET_BYTES, has_control, has_disallowed_text, required_string};
use crate::profiles::NodeProtocol;
use crate::sources::capabilities::Transport;
use serde_json::{Map, Value};

pub(crate) const SUPPORTED_FIELDS: &[&str] = &[
    "name",
    "type",
    "server",
    "port",
    "password",
    "ports",
    "hop-interval",
    "up",
    "down",
    "obfs",
    "obfs-password",
    "sni",
    "alpn",
    "skip-cert-verify",
    "fingerprint",
    "udp",
];

pub(crate) fn protocol_and_transport() -> (NodeProtocol, Transport) {
    (NodeProtocol::Hysteria2, Transport::Quic)
}

pub(crate) fn validate(object: &Map<String, Value>, node_index: usize) -> Result<(), ParserError> {
    let password = required_string(object, "password", node_index, MAX_SECRET_BYTES)?;
    if password.is_empty() || has_control(password) {
        return Err(ParserError::InvalidNode { node_index });
    }
    if let Some(ports) = object.get("ports") {
        let Some(ports) = ports.as_str() else {
            return Err(ParserError::InvalidNode { node_index });
        };
        validate_port_ranges(ports, node_index)?;
    }
    if let Some(interval) = object.get("hop-interval") {
        let Some(interval) = interval.as_str() else {
            return Err(ParserError::InvalidNode { node_index });
        };
        if interval.is_empty() || interval.len() > 32 || has_disallowed_text(interval) {
            return Err(ParserError::InvalidNode { node_index });
        }
    }
    for field in ["up", "down"] {
        if let Some(value) = object.get(field)
            && value
                .as_str()
                .is_none_or(|text| text.is_empty() || text.len() > 32 || has_disallowed_text(text))
        {
            return Err(ParserError::InvalidNode { node_index });
        }
    }
    if let Some(obfs) = object.get("obfs") {
        let Some(obfs) = obfs.as_str() else {
            return Err(ParserError::InvalidNode { node_index });
        };
        if obfs != "salamander" {
            return Err(ParserError::UnsupportedNodeFeature { node_index });
        }
        let secret = object
            .get("obfs-password")
            .and_then(Value::as_str)
            .unwrap_or("");
        if secret.is_empty() || has_control(secret) {
            return Err(ParserError::InvalidNode { node_index });
        }
    } else if object.contains_key("obfs-password") {
        return Err(ParserError::InvalidNode { node_index });
    }
    Ok(())
}

fn validate_port_ranges(text: &str, node_index: usize) -> Result<(), ParserError> {
    let parts: Vec<&str> = text
        .split([',', '/'])
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() || parts.len() > 28 {
        return Err(ParserError::InvalidNode { node_index });
    }
    for part in parts {
        if let Some((left, right)) = part.split_once('-') {
            let start: u32 = left
                .parse()
                .map_err(|_| ParserError::InvalidNode { node_index })?;
            let end: u32 = right
                .parse()
                .map_err(|_| ParserError::InvalidNode { node_index })?;
            if start > end || !(1..=65535).contains(&start) || !(1..=65535).contains(&end) {
                return Err(ParserError::InvalidNode { node_index });
            }
        } else {
            let port: u32 = part
                .parse()
                .map_err(|_| ParserError::InvalidNode { node_index })?;
            if !(1..=65535).contains(&port) {
                return Err(ParserError::InvalidNode { node_index });
            }
        }
    }
    Ok(())
}

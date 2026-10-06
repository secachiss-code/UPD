use super::super::ParserError;
use super::super::common::{MAX_SECRET_BYTES, has_control, optional_string};
use crate::profiles::NodeProtocol;
use crate::sources::capabilities::Transport;
use serde_json::Map;
use serde_json::Value;

pub(crate) const SUPPORTED_FIELDS: &[&str] = &[
    "name",
    "type",
    "server",
    "port",
    "username",
    "password",
    "udp",
    "tls",
    "servername",
    "sni",
    "alpn",
    "skip-cert-verify",
    "fingerprint",
    "name-cert-verify",
    "certificate",
    "private-key",
];

pub(crate) fn protocol_and_transport() -> (NodeProtocol, Transport) {
    (NodeProtocol::Socks5, Transport::Tcp)
}

pub(crate) fn validate(object: &Map<String, Value>, node_index: usize) -> Result<(), ParserError> {
    let username = optional_string(object, "username", node_index, MAX_SECRET_BYTES)?;
    let password = optional_string(object, "password", node_index, MAX_SECRET_BYTES)?;
    if username.is_some() != password.is_some()
        || username.is_some_and(|value| value.is_empty() || has_control(value))
        || password.is_some_and(|value| value.is_empty() || has_control(value))
    {
        return Err(ParserError::InvalidNode { node_index });
    }
    Ok(())
}

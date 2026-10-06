use super::super::ParserError;
use super::super::common::{MAX_SECRET_BYTES, has_control, required_string};
use crate::profiles::NodeProtocol;
use crate::sources::capabilities::Transport;
use serde_json::{Map, Value};

pub(crate) const SUPPORTED_FIELDS: &[&str] = &[
    "name",
    "type",
    "server",
    "port",
    "password",
    "sni",
    "alpn",
    "skip-cert-verify",
    "client-fingerprint",
    "fingerprint",
    "network",
    "udp",
    "reality-opts",
    "ws-opts",
    "grpc-opts",
    "tls",
    "servername",
];

pub(crate) fn protocol_and_transport(
    object: &Map<String, Value>,
    node_index: usize,
) -> Result<(NodeProtocol, Transport), ParserError> {
    let transport = super::super::transport::select_network(object, node_index, false)?;
    if !matches!(transport, Transport::Tcp | Transport::Ws | Transport::Grpc) {
        return Err(ParserError::UnsupportedTransport { node_index });
    }
    Ok((NodeProtocol::Trojan, transport))
}

pub(crate) fn validate(object: &Map<String, Value>, node_index: usize) -> Result<(), ParserError> {
    let password = required_string(object, "password", node_index, MAX_SECRET_BYTES)?;
    if password.is_empty() || has_control(password) {
        return Err(ParserError::InvalidNode { node_index });
    }
    if object.contains_key("reality-opts") {
        super::super::tls::validate_reality(object, node_index)?;
    }
    Ok(())
}

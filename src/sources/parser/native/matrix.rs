//! Allowed protocol × transport × security combinations.

use super::ParserError;
use crate::profiles::NodeProtocol;
use crate::sources::capabilities::Transport;
use serde_json::{Map, Value};

pub(crate) fn check(
    protocol: NodeProtocol,
    transport: Transport,
    object: &Map<String, Value>,
    node_index: usize,
) -> Result<(), ParserError> {
    let tls = object.get("tls").and_then(Value::as_bool) == Some(true)
        || matches!(protocol, NodeProtocol::Hysteria2 | NodeProtocol::Tuic);
    let reality = object.contains_key("reality-opts");
    if matches!(transport, Transport::H2 | Transport::Grpc) && !tls && !reality {
        return Err(ParserError::InvalidNode { node_index });
    }
    if reality
        && !matches!(
            transport,
            Transport::Tcp | Transport::Grpc | Transport::Xhttp
        )
    {
        return Err(ParserError::UnsupportedTransport { node_index });
    }
    if object.contains_key("flow") && transport != Transport::Tcp {
        return Err(ParserError::InvalidNode { node_index });
    }
    Ok(())
}

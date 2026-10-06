//! WireGuard nodes. Key length is checked at parse time (32 bytes after standard base64).

use super::super::ParserError;
use super::super::common::{has_disallowed_text, is_host};
use crate::profiles::NodeProtocol;
use crate::sources::capabilities::Transport;
use serde_json::{Map, Value};
use std::net::{Ipv4Addr, Ipv6Addr};

pub(crate) const SUPPORTED_FIELDS: &[&str] = &[
    "name",
    "type",
    "server",
    "port",
    "ip",
    "ipv6",
    "private-key",
    "public-key",
    "pre-shared-key",
    "allowed-ips",
    "mtu",
    "reserved",
    "peers",
    "persistent-keepalive",
    "udp",
];

const MAX_PEERS: usize = 64;

pub(crate) fn protocol_and_transport() -> (NodeProtocol, Transport) {
    (NodeProtocol::WireGuard, Transport::WireGuard)
}

pub(crate) fn validate(object: &Map<String, Value>, node_index: usize) -> Result<(), ParserError> {
    decode_key(object, "private-key", node_index, true)?;
    let peers = object.get("peers");
    let peer_list = match peers {
        None => None,
        Some(Value::Array(items)) => Some(items),
        Some(_) => return Err(ParserError::InvalidNode { node_index }),
    };
    // Without `peers` the top-level public key is required; with them it is optional.
    if peer_list.is_none() || object.contains_key("public-key") {
        decode_key(object, "public-key", node_index, true)?;
    }
    if object.contains_key("pre-shared-key") {
        decode_key(object, "pre-shared-key", node_index, false)?;
    }
    require_cidr(object, "ip", node_index, true)?;
    if object.contains_key("ipv6") {
        require_cidr(object, "ipv6", node_index, true)?;
    }
    let mut ranges = Vec::new();
    if let Some(list) = object.get("allowed-ips") {
        ranges.extend(parse_cidr_list(list, node_index)?);
    } else if peer_list.is_none() {
        return Err(ParserError::InvalidNode { node_index });
    }
    if let Some(mtu) = object.get("mtu")
        && mtu.as_u64().is_none_or(|value| value > 65535)
    {
        return Err(ParserError::InvalidNode { node_index });
    }
    if object.contains_key("reserved") {
        validate_reserved(object.get("reserved"), node_index)?;
    }
    if let Some(keepalive) = object.get("persistent-keepalive")
        && keepalive.as_u64().is_none_or(|value| value > 65535)
    {
        return Err(ParserError::InvalidNode { node_index });
    }
    if let Some(peers) = peer_list {
        if peers.len() > MAX_PEERS {
            return Err(ParserError::InvalidNode { node_index });
        }
        let mut peer_ranges = Vec::new();
        for peer in peers {
            let Some(peer) = peer.as_object() else {
                return Err(ParserError::InvalidNode { node_index });
            };
            decode_key(peer, "public-key", node_index, true)?;
            if peer.contains_key("pre-shared-key") {
                decode_key(peer, "pre-shared-key", node_index, false)?;
            }
            let Some(server) = peer.get("server").and_then(Value::as_str) else {
                return Err(ParserError::InvalidNode { node_index });
            };
            if !is_host(server) {
                return Err(ParserError::InvalidNode { node_index });
            }
            if peer
                .get("port")
                .and_then(Value::as_u64)
                .is_none_or(|port| !(1..=u16::MAX as u64).contains(&port))
            {
                return Err(ParserError::InvalidNode { node_index });
            }
            if peer.contains_key("reserved") {
                validate_reserved(peer.get("reserved"), node_index)?;
            }
            let Some(allowed) = peer.get("allowed-ips") else {
                return Err(ParserError::InvalidNode { node_index });
            };
            peer_ranges.extend(parse_cidr_list(allowed, node_index)?);
        }
        if ranges_overlap(&peer_ranges) {
            return Err(ParserError::InvalidNode { node_index });
        }
    }
    if ranges_overlap(&ranges) {
        return Err(ParserError::InvalidNode { node_index });
    }
    Ok(())
}

fn decode_key(
    object: &Map<String, Value>,
    field: &str,
    node_index: usize,
    required: bool,
) -> Result<(), ParserError> {
    match object.get(field) {
        None if required => Err(ParserError::InvalidNode { node_index }),
        None => Ok(()),
        Some(Value::String(text)) => {
            if has_disallowed_text(text) {
                return Err(ParserError::InvalidNode { node_index });
            }
            match decode_b64_std(text) {
                Some(bytes) if bytes.len() == 32 => Ok(()),
                _ => Err(ParserError::InvalidNode { node_index }),
            }
        }
        Some(_) => Err(ParserError::InvalidNode { node_index }),
    }
}

fn require_cidr(
    object: &Map<String, Value>,
    field: &str,
    node_index: usize,
    required: bool,
) -> Result<(), ParserError> {
    match object.get(field) {
        None if required => Err(ParserError::InvalidNode { node_index }),
        None => Ok(()),
        Some(value) => {
            parse_cidr_list(&Value::Array(vec![value.clone()]), node_index)?;
            Ok(())
        }
    }
}

fn parse_cidr_list(value: &Value, node_index: usize) -> Result<Vec<(u128, u128)>, ParserError> {
    let Value::Array(items) = value else {
        return Err(ParserError::InvalidNode { node_index });
    };
    if items.is_empty() || items.len() > 64 {
        return Err(ParserError::InvalidNode { node_index });
    }
    let mut ranges = Vec::with_capacity(items.len());
    for item in items {
        let Some(text) = item.as_str() else {
            return Err(ParserError::InvalidNode { node_index });
        };
        ranges.push(parse_cidr(text).ok_or(ParserError::InvalidNode { node_index })?);
    }
    Ok(ranges)
}

fn parse_cidr(text: &str) -> Option<(u128, u128)> {
    let (addr, prefix) = text.split_once('/')?;
    let prefix: u32 = prefix.parse().ok()?;
    if let Ok(ip) = addr.parse::<Ipv4Addr>() {
        if prefix > 32 {
            return None;
        }
        let ip = u32::from(ip);
        let mask = if prefix == 0 {
            0
        } else {
            u32::MAX << (32 - prefix)
        };
        let start = ip & mask;
        let end = start | !mask;
        return Some((start as u128, end as u128));
    }
    if let Ok(ip) = addr.parse::<Ipv6Addr>() {
        if prefix > 128 {
            return None;
        }
        let ip = u128::from(ip);
        let mask = if prefix == 0 {
            0
        } else {
            u128::MAX << (128 - prefix)
        };
        let start = ip & mask;
        let end = start | !mask;
        return Some((start, end));
    }
    None
}

fn ranges_overlap(ranges: &[(u128, u128)]) -> bool {
    for (index, left) in ranges.iter().enumerate() {
        for right in &ranges[index + 1..] {
            if left.0 <= right.1 && right.0 <= left.1 {
                return true;
            }
        }
    }
    false
}

fn validate_reserved(value: Option<&Value>, node_index: usize) -> Result<(), ParserError> {
    let Some(Value::Array(items)) = value else {
        return Err(ParserError::InvalidNode { node_index });
    };
    if items.len() != 3
        || items
            .iter()
            .any(|item| item.as_u64().is_none_or(|byte| byte > 255))
    {
        return Err(ParserError::InvalidNode { node_index });
    }
    Ok(())
}

fn decode_b64_std(text: &str) -> Option<Vec<u8>> {
    fn val(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes = text.as_bytes();
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let pad = chunk.iter().rev().take_while(|byte| **byte == b'=').count();
        if pad > 2 {
            return None;
        }
        let mut acc = 0u32;
        for (index, byte) in chunk.iter().enumerate() {
            if *byte == b'=' {
                if index < 2 || chunk[index..].iter().any(|item| *item != b'=') {
                    return None;
                }
                continue;
            }
            acc = (acc << 6) | u32::from(val(*byte)?);
        }
        acc <<= 6 * pad as u32;
        out.push((acc >> 16) as u8);
        if pad < 2 {
            out.push((acc >> 8) as u8);
        }
        if pad < 1 {
            out.push(acc as u8);
        }
    }
    Some(out)
}

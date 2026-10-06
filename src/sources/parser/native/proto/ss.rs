use super::super::ParserError;
use super::super::common::{
    MAX_SECRET_BYTES, has_control, is_supported_ss_cipher, required_string,
};
use crate::profiles::NodeProtocol;
use crate::sources::capabilities::Transport;
use serde_json::Map;
use serde_json::Value;

pub(crate) const SUPPORTED_FIELDS: &[&str] = &[
    "name",
    "type",
    "server",
    "port",
    "password",
    "cipher",
    "udp",
    "udp-over-tcp",
    "udp-over-tcp-version",
    "client-fingerprint",
];

pub(crate) fn protocol_and_transport() -> (NodeProtocol, Transport) {
    (NodeProtocol::Shadowsocks, Transport::Tcp)
}

pub(crate) fn validate(object: &Map<String, Value>, node_index: usize) -> Result<(), ParserError> {
    let password = required_string(object, "password", node_index, MAX_SECRET_BYTES)?;
    if password.is_empty() || has_control(password) {
        return Err(ParserError::InvalidNode { node_index });
    }
    let cipher = required_string(object, "cipher", node_index, 64)?;
    if let Some(key_len) = cipher_2022_key_len(cipher) {
        validate_2022_password(
            password,
            key_len,
            cipher.ends_with("chacha20-poly1305") || cipher.ends_with("chacha8-poly1305"),
            node_index,
        )?;
    } else if !is_supported_ss_cipher(cipher) {
        return Err(ParserError::InvalidNode { node_index });
    }
    if object.contains_key("udp-over-tcp") {
        // Must be a bool; without an explicit version the pinned core uses version 1.
        object
            .get("udp-over-tcp")
            .and_then(Value::as_bool)
            .ok_or(ParserError::InvalidNode { node_index })?;
        if let Some(version) = object.get("udp-over-tcp-version") {
            let version = version
                .as_u64()
                .ok_or(ParserError::InvalidNode { node_index })?;
            if !matches!(version, 0..=2) {
                return Err(ParserError::InvalidNode { node_index });
            }
        }
    } else if object.contains_key("udp-over-tcp-version") {
        return Err(ParserError::InvalidNode { node_index });
    }
    if let Some(fingerprint) = object.get("client-fingerprint") {
        let Some(name) = fingerprint.as_str() else {
            return Err(ParserError::InvalidNode { node_index });
        };
        if name.is_empty() || has_control(name) {
            return Err(ParserError::InvalidNode { node_index });
        }
    }
    Ok(())
}

fn cipher_2022_key_len(cipher: &str) -> Option<usize> {
    match cipher {
        "2022-blake3-aes-128-gcm" | "2022-blake3-aes-128-ccm" => Some(16),
        "2022-blake3-aes-256-gcm"
        | "2022-blake3-aes-256-ccm"
        | "2022-blake3-chacha20-poly1305"
        | "2022-blake3-chacha8-poly1305" => Some(32),
        _ => None,
    }
}

fn validate_2022_password(
    password: &str,
    key_len: usize,
    single_key: bool,
    node_index: usize,
) -> Result<(), ParserError> {
    let parts: Vec<&str> = password.split(':').collect();
    if parts.is_empty() || (single_key && parts.len() != 1) {
        return Err(ParserError::InvalidNode { node_index });
    }
    for part in parts {
        let Some(bytes) = decode_b64_std(part) else {
            return Err(ParserError::InvalidNode { node_index });
        };
        if bytes.len() != key_len {
            return Err(ParserError::InvalidNode { node_index });
        }
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
    let mut out = Vec::new();
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

//! TLS options for pinned mihomo v1.19.32 and D2 `tls_verification`.

use super::ParserError;
use super::common::{bool_value, has_disallowed_text, is_host, optional_string};
use crate::profiles::TlsVerification;
use serde_json::{Map, Value};

#[allow(dead_code)] // protocol allow-lists repeat these names until I03.T04.i
pub(crate) const TLS_FIELDS: &[&str] = &[
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
];

const CLIENT_FINGERPRINTS: &[&str] = &[
    "chrome",
    "firefox",
    "safari",
    "ios",
    "android",
    "edge",
    "360",
    "qq",
    "random",
    "chrome120",
    "firefox120",
    "safari16",
    "chrome_psk",
    "chrome_psk_shuffle",
    "chrome_padding_psk_shuffle",
    "chrome_pq",
    "chrome_pq_psk",
    "randomized",
    "none",
];

/// Validate TLS-related keys and classify how the TLS hop to the proxy is authenticated.
///
/// `tls_hop` says whether this node actually has a TLS hop (Trojan/Hysteria2/TUIC always,
/// VLESS/VMess/HTTP/SOCKS5 only with `tls: true`). Without one the result is
/// `NotApplicable`: a plaintext channel is never reported as verified.
pub(crate) fn classify(
    object: &Map<String, Value>,
    node_index: usize,
    reject_host_certs: bool,
    tls_hop: bool,
) -> Result<TlsVerification, ParserError> {
    if reject_host_certs {
        for field in ["certificate", "private-key"] {
            if let Some(value) = object.get(field).and_then(Value::as_str) {
                let pem = value.contains("-----BEGIN ");
                let path = value.starts_with('/') || value.contains("..");
                if path || !pem {
                    return Err(ParserError::RestrictedNodeOption { node_index });
                }
            } else if object.contains_key(field) {
                return Err(ParserError::InvalidNode { node_index });
            }
        }
    }
    if object.contains_key("tls") {
        bool_value(object, "tls", node_index)?;
    }
    let servername = optional_string(object, "servername", node_index, 253)?;
    let sni = optional_string(object, "sni", node_index, 253)?;
    if let Some(name) = servername.or(sni)
        && !name.is_empty()
        && !is_host(name)
    {
        return Err(ParserError::InvalidNode { node_index });
    }
    if let (Some(left), Some(right)) = (servername, sni)
        && left != right
    {
        return Err(ParserError::InvalidNode { node_index });
    }
    if object.contains_key("alpn") {
        validate_alpn(object.get("alpn"), node_index)?;
    }
    if let Some(name) = optional_string(object, "client-fingerprint", node_index, 64)?
        && !CLIENT_FINGERPRINTS.contains(&name)
    {
        return Err(ParserError::InvalidNode { node_index });
    }
    let fingerprint = optional_string(object, "fingerprint", node_index, 64)?;
    if let Some(pin) = fingerprint
        && !is_sha256_hex(pin)
    {
        return Err(ParserError::InvalidNode { node_index });
    }
    let name_pin = optional_string(object, "name-cert-verify", node_index, 256)?;
    if name_pin.is_some_and(|value| value.is_empty() || has_disallowed_text(value)) {
        return Err(ParserError::InvalidNode { node_index });
    }
    let skip = match object.get("skip-cert-verify") {
        None => false,
        Some(_) => bool_value(object, "skip-cert-verify", node_index)?,
    };
    // The pinned core verifies a `fingerprint`/`name-cert-verify` pin with its own verifier
    // whether or not `skip-cert-verify` is set; REALITY authenticates by the server public key.
    let pinned = fingerprint.is_some() || name_pin.is_some() || object.contains_key("reality-opts");
    Ok(match (tls_hop, pinned, skip) {
        (false, _, _) => TlsVerification::NotApplicable,
        (true, true, _) => TlsVerification::Pinned,
        (true, false, true) => TlsVerification::Disabled,
        (true, false, false) => TlsVerification::Verified,
    })
}

/// Whether the node has a TLS hop to the proxy server.
pub(crate) fn has_tls_hop(
    protocol: crate::profiles::NodeProtocol,
    object: &Map<String, Value>,
) -> bool {
    use crate::profiles::NodeProtocol;
    match protocol {
        NodeProtocol::Trojan | NodeProtocol::Hysteria2 | NodeProtocol::Tuic => true,
        NodeProtocol::Vless | NodeProtocol::Vmess | NodeProtocol::Http | NodeProtocol::Socks5 => {
            object.get("tls").and_then(Value::as_bool) == Some(true)
        }
        _ => false,
    }
}

fn validate_alpn(value: Option<&Value>, node_index: usize) -> Result<(), ParserError> {
    let Some(Value::Array(items)) = value else {
        return Err(ParserError::InvalidNode { node_index });
    };
    if items.len() > 16 {
        return Err(ParserError::InvalidNode { node_index });
    }
    for item in items {
        let Some(text) = item.as_str() else {
            return Err(ParserError::InvalidNode { node_index });
        };
        if text.is_empty() || text.len() > 64 || has_disallowed_text(text) {
            return Err(ParserError::InvalidNode { node_index });
        }
    }
    Ok(())
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// REALITY block. `public-key` is base64url of 32 bytes. `short-id` is even-length hex, at most 16 chars.
pub(crate) fn validate_reality(
    object: &Map<String, Value>,
    node_index: usize,
) -> Result<(), ParserError> {
    let network = object
        .get("network")
        .and_then(Value::as_str)
        .unwrap_or("tcp");
    if !matches!(network, "tcp" | "grpc" | "xhttp") {
        return Err(ParserError::UnsupportedTransport { node_index });
    }
    // Pinned core: VLESS refuses REALITY without `tls: true`; Trojan is always TLS.
    let trojan = object.get("type").and_then(Value::as_str) == Some("trojan");
    if !trojan && object.get("tls").and_then(Value::as_bool) != Some(true) {
        return Err(ParserError::InvalidNode { node_index });
    }
    // REALITY runs on uTLS: without a usable `client-fingerprint` the core refuses the
    // connection at dial time ("REALITY is based on uTLS"). Refuse at import instead.
    let usable_fingerprint = object
        .get("client-fingerprint")
        .and_then(Value::as_str)
        .is_some_and(|name| name != "none" && CLIENT_FINGERPRINTS.contains(&name));
    if !usable_fingerprint {
        return Err(ParserError::InvalidNode { node_index });
    }
    let servername = object
        .get("servername")
        .or_else(|| object.get("sni"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if servername.is_empty() {
        return Err(ParserError::InvalidNode { node_index });
    }
    let Some(Value::Object(opts)) = object.get("reality-opts") else {
        return Err(ParserError::InvalidNode { node_index });
    };
    const KEYS: &[&str] = &["public-key", "short-id", "support-x25519mlkem768"];
    if opts.keys().any(|key| !KEYS.contains(&key.as_str())) {
        return Err(ParserError::UnsupportedNodeField { node_index });
    }
    let Some(public_key) = opts.get("public-key").and_then(Value::as_str) else {
        return Err(ParserError::InvalidNode { node_index });
    };
    if decode_b64_url(public_key).is_none_or(|bytes| bytes.len() != 32) {
        return Err(ParserError::InvalidNode { node_index });
    }
    if let Some(short_id) = opts.get("short-id") {
        let Some(short_id) = short_id.as_str() else {
            return Err(ParserError::InvalidNode { node_index });
        };
        if short_id.len() > 16
            || short_id.len() % 2 != 0
            || !short_id.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(ParserError::InvalidNode { node_index });
        }
    }
    if opts.contains_key("support-x25519mlkem768")
        && opts
            .get("support-x25519mlkem768")
            .and_then(Value::as_bool)
            .is_none()
    {
        return Err(ParserError::InvalidNode { node_index });
    }
    Ok(())
}

fn decode_b64_url(text: &str) -> Option<Vec<u8>> {
    fn val(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    }
    let bytes = text.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return None;
    }
    let mut acc = 0u32;
    let mut bits = 0;
    let mut out = Vec::new();
    for byte in bytes {
        // RawURLEncoding: no padding, nothing after it.
        acc = (acc << 6) | u32::from(val(*byte)?);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

//! Transport selection and HTTP header validation for native proxies.

use super::ParserError;
use super::common::has_disallowed_text;
use crate::sources::capabilities::Transport;
use serde_json::{Map, Value};
use std::collections::BTreeSet;

/// Select `network` and validate the matching `*-opts` object.
pub(crate) fn select_network(
    object: &Map<String, Value>,
    node_index: usize,
    allow_xhttp: bool,
) -> Result<Transport, ParserError> {
    let transport = match object.get("network") {
        None => Transport::Tcp,
        Some(Value::String(network)) => match network.as_str() {
            "tcp" => Transport::Tcp,
            "ws" => Transport::Ws,
            "http" => Transport::Http,
            "h2" => Transport::H2,
            "grpc" => Transport::Grpc,
            "xhttp" if allow_xhttp => Transport::Xhttp,
            "xhttp" => return Err(ParserError::UnsupportedTransport { node_index }),
            _ => return Err(ParserError::UnsupportedTransport { node_index }),
        },
        Some(_) => return Err(ParserError::InvalidNode { node_index }),
    };
    validate_transport_opts(object, transport, node_index)?;
    Ok(transport)
}

fn validate_transport_opts(
    object: &Map<String, Value>,
    transport: Transport,
    node_index: usize,
) -> Result<(), ParserError> {
    let expected = match transport {
        Transport::Ws => Some("ws-opts"),
        Transport::Http => Some("http-opts"),
        Transport::H2 => Some("h2-opts"),
        Transport::Grpc => Some("grpc-opts"),
        Transport::Xhttp => Some("xhttp-opts"),
        _ => None,
    };
    for key in ["ws-opts", "http-opts", "h2-opts", "grpc-opts", "xhttp-opts"] {
        if object.contains_key(key) && Some(key) != expected {
            return Err(ParserError::InvalidNode { node_index });
        }
    }
    let Some(key) = expected else {
        return Ok(());
    };
    let Some(value) = object.get(key) else {
        return Ok(());
    };
    let Value::Object(opts) = value else {
        return Err(ParserError::InvalidNode { node_index });
    };
    match transport {
        Transport::Ws => validate_ws_opts(opts, node_index),
        Transport::Http => validate_http_opts(opts, node_index),
        Transport::H2 => validate_h2_opts(opts, node_index),
        Transport::Grpc => validate_grpc_opts(opts, node_index),
        Transport::Xhttp => validate_xhttp_opts(opts, node_index),
        _ => Ok(()),
    }
}

fn validate_ws_opts(opts: &Map<String, Value>, node_index: usize) -> Result<(), ParserError> {
    const KEYS: &[&str] = &[
        "path",
        "headers",
        "max-early-data",
        "early-data-header-name",
        "v2ray-http-upgrade",
        "v2ray-http-upgrade-fast-open",
    ];
    reject_unknown_keys(opts, KEYS, node_index)?;
    if let Some(path) = opts.get("path") {
        let Some(path) = path.as_str() else {
            return Err(ParserError::InvalidNode { node_index });
        };
        if (!path.is_empty() && !path.starts_with('/'))
            || path.len() > 2048
            || has_disallowed_text(path)
        {
            return Err(ParserError::InvalidNode { node_index });
        }
    }
    if opts.contains_key("headers") {
        validate_headers(opts.get("headers"), node_index)?;
    }
    if opts.contains_key("max-early-data") && opts["max-early-data"].as_u64().is_none() {
        return Err(ParserError::InvalidNode { node_index });
    }
    let upgrade = opts.get("v2ray-http-upgrade").and_then(Value::as_bool);
    let fast_open = opts
        .get("v2ray-http-upgrade-fast-open")
        .and_then(Value::as_bool);
    if opts.contains_key("v2ray-http-upgrade") && upgrade.is_none()
        || opts.contains_key("v2ray-http-upgrade-fast-open") && fast_open.is_none()
    {
        return Err(ParserError::InvalidNode { node_index });
    }
    if upgrade == Some(true) && fast_open == Some(true) {
        return Err(ParserError::InvalidNode { node_index });
    }
    Ok(())
}

fn validate_http_opts(opts: &Map<String, Value>, node_index: usize) -> Result<(), ParserError> {
    const KEYS: &[&str] = &["method", "path", "headers"];
    reject_unknown_keys(opts, KEYS, node_index)?;
    if let Some(method) = opts.get("method") {
        let Some(method) = method.as_str() else {
            return Err(ParserError::InvalidNode { node_index });
        };
        if method.is_empty() || !method.bytes().all(is_http_token_byte) {
            return Err(ParserError::InvalidNode { node_index });
        }
    }
    if let Some(paths) = opts.get("path") {
        require_string_list(paths, node_index, true)?;
    }
    if let Some(headers) = opts.get("headers") {
        validate_header_lists(headers, node_index)?;
    }
    Ok(())
}

fn validate_h2_opts(opts: &Map<String, Value>, node_index: usize) -> Result<(), ParserError> {
    const KEYS: &[&str] = &["host", "path"];
    reject_unknown_keys(opts, KEYS, node_index)?;
    if let Some(hosts) = opts.get("host") {
        require_string_list(hosts, node_index, false)?;
    }
    if let Some(path) = opts.get("path") {
        let Some(path) = path.as_str() else {
            return Err(ParserError::InvalidNode { node_index });
        };
        if (!path.is_empty() && !path.starts_with('/'))
            || path.len() > 2048
            || has_disallowed_text(path)
        {
            return Err(ParserError::InvalidNode { node_index });
        }
    }
    Ok(())
}

fn validate_grpc_opts(opts: &Map<String, Value>, node_index: usize) -> Result<(), ParserError> {
    const KEYS: &[&str] = &["grpc-service-name"];
    reject_unknown_keys(opts, KEYS, node_index)?;
    if let Some(name) = opts.get("grpc-service-name") {
        let Some(name) = name.as_str() else {
            return Err(ParserError::InvalidNode { node_index });
        };
        if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
            return Err(ParserError::InvalidNode { node_index });
        }
    }
    Ok(())
}

fn validate_xhttp_opts(opts: &Map<String, Value>, node_index: usize) -> Result<(), ParserError> {
    const KEYS: &[&str] = &["path", "host", "mode", "headers"];
    reject_unknown_keys(opts, KEYS, node_index)?;
    for key in ["path", "host", "mode"] {
        if let Some(value) = opts.get(key)
            && value
                .as_str()
                .is_none_or(|text| text.len() > 2048 || has_disallowed_text(text))
        {
            return Err(ParserError::InvalidNode { node_index });
        }
    }
    // Pinned core passes unknown modes through unchecked; accept only the modes it implements.
    if let Some(mode) = opts.get("mode").and_then(Value::as_str)
        && !matches!(mode, "" | "auto" | "packet-up" | "stream-up" | "stream-one")
    {
        return Err(ParserError::UnsupportedNodeFeature { node_index });
    }
    if opts.contains_key("headers") {
        validate_headers(opts.get("headers"), node_index)?;
    }
    Ok(())
}

fn reject_unknown_keys(
    opts: &Map<String, Value>,
    allowed: &[&str],
    node_index: usize,
) -> Result<(), ParserError> {
    if opts.keys().any(|key| !allowed.contains(&key.as_str())) {
        Err(ParserError::UnsupportedNodeField { node_index })
    } else {
        Ok(())
    }
}

fn require_string_list(value: &Value, node_index: usize, paths: bool) -> Result<(), ParserError> {
    let Value::Array(items) = value else {
        return Err(ParserError::InvalidNode { node_index });
    };
    if items.is_empty() || items.len() > 32 {
        return Err(ParserError::InvalidNode { node_index });
    }
    for item in items {
        let Some(text) = item.as_str() else {
            return Err(ParserError::InvalidNode { node_index });
        };
        if text.is_empty()
            || text.len() > 2048
            || has_disallowed_text(text)
            || (paths && !text.starts_with('/'))
        {
            return Err(ParserError::InvalidNode { node_index });
        }
    }
    Ok(())
}

fn validate_header_lists(value: &Value, node_index: usize) -> Result<(), ParserError> {
    let Value::Object(headers) = value else {
        return Err(ParserError::InvalidNode { node_index });
    };
    if headers.len() > 128 {
        return Err(ParserError::InvalidNode { node_index });
    }
    let mut normalized_names = BTreeSet::new();
    for (name, values) in headers {
        if name.is_empty()
            || name.len() > 256
            || !name.bytes().all(is_http_token_byte)
            || !normalized_names.insert(name.to_ascii_lowercase())
        {
            return Err(ParserError::InvalidNode { node_index });
        }
        let Value::Array(items) = values else {
            return Err(ParserError::InvalidNode { node_index });
        };
        if items.is_empty() || items.len() > 32 {
            return Err(ParserError::InvalidNode { node_index });
        }
        for item in items {
            let Some(text) = item.as_str() else {
                return Err(ParserError::InvalidNode { node_index });
            };
            if text.len() > 8192
                || text
                    .bytes()
                    .any(|byte| byte != b'\t' && (byte < 0x20 || byte == 0x7f))
            {
                return Err(ParserError::InvalidNode { node_index });
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_headers(
    value: Option<&Value>,
    node_index: usize,
) -> Result<(), ParserError> {
    let Some(Value::Object(headers)) = value else {
        return Err(ParserError::InvalidNode { node_index });
    };
    if headers.len() > 128 {
        return Err(ParserError::InvalidNode { node_index });
    }
    let mut normalized_names = BTreeSet::new();
    for (name, value) in headers {
        if name.is_empty()
            || name.len() > 256
            || !name.bytes().all(is_http_token_byte)
            || !normalized_names.insert(name.to_ascii_lowercase())
            || value.as_str().is_none_or(|text| {
                text.len() > 8192
                    || text
                        .bytes()
                        .any(|byte| byte != b'\t' && (byte < 0x20 || byte == 0x7f))
            })
        {
            return Err(ParserError::InvalidNode { node_index });
        }
    }
    Ok(())
}

fn is_http_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

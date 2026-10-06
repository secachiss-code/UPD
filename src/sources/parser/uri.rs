//! Share-link parser. One line becomes the same node object as the native subset.
//!
//! Every query parameter is either mapped onto the native field it means, or refused:
//! an unknown parameter is `UnsupportedNodeField`, a known one with a value outside the
//! supported subset is `UnsupportedNodeFeature`. Nothing is dropped — in particular a
//! `security` mode is never silently downgraded to plaintext. The produced object then
//! goes through the same native node validator as YAML/JSON sources.

use super::native::ParserError;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// Same ceiling as one configured subscription URL.
const MAX_URI_BYTES: usize = 8192;

const MALFORMED: ParserError = ParserError::InvalidNode { node_index: 0 };
const UNSUPPORTED_FIELD: ParserError = ParserError::UnsupportedNodeField { node_index: 0 };
const UNSUPPORTED_FEATURE: ParserError = ParserError::UnsupportedNodeFeature { node_index: 0 };

pub fn parse_share_uri(line: &str) -> Result<Value, ParserError> {
    let line = line.trim();
    if line.len() > MAX_URI_BYTES {
        return Err(ParserError::UriTooLong);
    }
    let (scheme, rest) = line.split_once("://").ok_or(MALFORMED)?;
    let scheme = scheme.to_ascii_lowercase();
    match scheme.as_str() {
        "vmess" => return parse_vmess(rest),
        "ss" => return parse_ss(rest),
        "vless" | "trojan" | "hysteria2" | "hy2" | "tuic" | "socks5" | "socks" | "http"
        | "https" => {}
        _ => return Err(ParserError::UnsupportedProtocol { node_index: 0 }),
    }
    let url = url::Url::parse(line).map_err(|_| MALFORMED)?;
    let name = percent_decode(url.fragment().unwrap_or(""))?;
    if name.is_empty() {
        return Err(MALFORMED);
    }
    let host = url
        .host_str()
        .ok_or(MALFORMED)?
        .trim_matches(|ch| ch == '[' || ch == ']')
        .to_owned();
    let port = url.port_or_known_default().ok_or(MALFORMED)?;
    if port == 0 {
        return Err(MALFORMED);
    }
    let mut query = Query::parse(url.query().unwrap_or(""))?;
    let mut node = Map::new();
    node.insert("name".into(), Value::String(name));
    node.insert("server".into(), Value::String(host));
    node.insert("port".into(), json!(port));
    let username = percent_decode(url.username())?;
    let password = url.password().map(percent_decode).transpose()?;
    match scheme.as_str() {
        "vless" => {
            node.insert("type".into(), json!("vless"));
            node.insert("uuid".into(), Value::String(username));
            if password.is_some() {
                return Err(MALFORMED);
            }
            if let Some(encryption) = query.take("encryption")
                && encryption != "none"
            {
                return Err(UNSUPPORTED_FEATURE);
            }
            let network = query.take("type").unwrap_or_else(|| "tcp".into());
            node.insert("network".into(), json!(network));
            let security = query.take("security").unwrap_or_default();
            match security.as_str() {
                "" | "none" => {
                    node.insert("tls".into(), Value::Bool(false));
                }
                "tls" => {
                    node.insert("tls".into(), Value::Bool(true));
                }
                "reality" => {
                    node.insert("tls".into(), Value::Bool(true));
                    reality(&mut node, &mut query)?;
                }
                _ => return Err(UNSUPPORTED_FEATURE),
            }
            if security != "reality" && (query.has("pbk") || query.has("sid")) {
                return Err(MALFORMED);
            }
            tls_params(&mut node, &mut query, "servername")?;
            if let Some(flow) = query.take("flow") {
                node.insert("flow".into(), Value::String(flow));
            }
            if let Some(encoding) = query.take("packetEncoding") {
                node.insert("packet-encoding".into(), Value::String(encoding));
            }
            transport_params(&mut node, &mut query, &network)?;
        }
        "trojan" => {
            node.insert("type".into(), json!("trojan"));
            let secret = password.unwrap_or(username);
            node.insert("password".into(), Value::String(secret));
            let network = query.take("type").unwrap_or_else(|| "tcp".into());
            node.insert("network".into(), json!(network));
            match query.take("security").as_deref() {
                None | Some("") | Some("tls") => {}
                Some("reality") => reality(&mut node, &mut query)?,
                Some(_) => return Err(UNSUPPORTED_FEATURE),
            }
            if let Some(peer) = query.take("peer") {
                if query.has("sni") {
                    return Err(ParserError::DuplicateKey);
                }
                node.insert("sni".into(), Value::String(peer));
            }
            tls_params(&mut node, &mut query, "sni")?;
            transport_params(&mut node, &mut query, &network)?;
        }
        "hysteria2" | "hy2" => {
            node.insert("type".into(), json!("hysteria2"));
            let secret = match password {
                Some(password) => format!("{username}:{password}"),
                None => username,
            };
            node.insert("password".into(), Value::String(secret));
            if let Some(obfs) = query.take("obfs") {
                node.insert("obfs".into(), Value::String(obfs));
            }
            if let Some(secret) = query.take("obfs-password") {
                node.insert("obfs-password".into(), Value::String(secret));
            }
            if let Some(ports) = query.take("mport") {
                node.insert("ports".into(), Value::String(ports));
            }
            if query.take("pinSHA256").is_some() {
                // Hysteria pins use a different encoding than the core's `fingerprint`.
                return Err(UNSUPPORTED_FEATURE);
            }
            tls_params(&mut node, &mut query, "sni")?;
        }
        "tuic" => {
            node.insert("type".into(), json!("tuic"));
            node.insert("uuid".into(), Value::String(username));
            node.insert("password".into(), Value::String(password.ok_or(MALFORMED)?));
            if let Some(value) = query.take("congestion_control") {
                node.insert("congestion-controller".into(), Value::String(value));
            }
            if let Some(value) = query.take("udp_relay_mode") {
                node.insert("udp-relay-mode".into(), Value::String(value));
            }
            for (param, field) in [("disable_sni", "disable-sni"), ("reduce_rtt", "reduce-rtt")] {
                if let Some(value) = query.take(param) {
                    node.insert(field.into(), Value::Bool(flag(&value)?));
                }
            }
            if let Some(value) = query.take("allow_insecure") {
                if query.has("insecure") {
                    return Err(ParserError::DuplicateKey);
                }
                node.insert("skip-cert-verify".into(), Value::Bool(flag(&value)?));
            }
            tls_params(&mut node, &mut query, "sni")?;
        }
        "socks5" | "socks" | "http" | "https" => {
            let kind = if scheme.starts_with("socks") {
                "socks5"
            } else {
                "http"
            };
            node.insert("type".into(), json!(kind));
            match (username.is_empty(), password) {
                (true, None) => {}
                (false, Some(password)) => {
                    node.insert("username".into(), Value::String(username));
                    node.insert("password".into(), Value::String(password));
                }
                _ => return Err(MALFORMED),
            }
            if kind == "http" {
                node.insert("tls".into(), Value::Bool(scheme == "https"));
            }
        }
        _ => unreachable!("scheme filtered above"),
    }
    query.finish()?;
    Ok(Value::Object(node))
}

/// Query parameters with duplicate detection; every parameter must be consumed.
struct Query(BTreeMap<String, String>);

impl Query {
    fn parse(query: &str) -> Result<Self, ParserError> {
        let mut map = BTreeMap::new();
        for pair in query.split('&').filter(|pair| !pair.is_empty()) {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let key = percent_decode(key)?;
            let value = percent_decode(value)?;
            if map.insert(key, value).is_some() {
                return Err(ParserError::DuplicateKey);
            }
        }
        Ok(Self(map))
    }

    fn take(&mut self, key: &str) -> Option<String> {
        self.0.remove(key)
    }

    fn has(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    /// Any parameter left over has no mapping: refuse instead of dropping it.
    fn finish(self) -> Result<(), ParserError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(UNSUPPORTED_FIELD)
        }
    }
}

/// `sni`, `fp`, `alpn` and D2 `insecure`/`allowInsecure` → native TLS fields.
fn tls_params(
    node: &mut Map<String, Value>,
    query: &mut Query,
    sni_field: &str,
) -> Result<(), ParserError> {
    if let Some(sni) = query.take("sni") {
        node.insert(sni_field.into(), Value::String(sni));
    }
    if let Some(fingerprint) = query.take("fp") {
        node.insert("client-fingerprint".into(), Value::String(fingerprint));
    }
    if let Some(alpn) = query.take("alpn") {
        node.insert("alpn".into(), alpn_list(&alpn)?);
    }
    let insecure = match (query.take("insecure"), query.take("allowInsecure")) {
        (Some(_), Some(_)) => return Err(ParserError::DuplicateKey),
        (Some(value), None) | (None, Some(value)) => Some(flag(&value)?),
        (None, None) => None,
    };
    // Only an explicit parameter sets the field (I03-DECISIONS D2); never an implicit true.
    if let Some(insecure) = insecure {
        node.insert("skip-cert-verify".into(), Value::Bool(insecure));
    }
    Ok(())
}

fn reality(node: &mut Map<String, Value>, query: &mut Query) -> Result<(), ParserError> {
    let public_key = query.take("pbk").ok_or(MALFORMED)?;
    let mut opts = Map::new();
    opts.insert("public-key".into(), Value::String(public_key));
    if let Some(short_id) = query.take("sid") {
        opts.insert("short-id".into(), Value::String(short_id));
    }
    if query
        .take("spx")
        .is_some_and(|spider| !spider.is_empty() && spider != "/")
    {
        // The pinned core has no spider-x option.
        return Err(UNSUPPORTED_FEATURE);
    }
    node.insert("reality-opts".into(), Value::Object(opts));
    Ok(())
}

/// `host`, `path`, `serviceName`, `mode`, `headerType` → the `*-opts` of the chosen network.
fn transport_params(
    node: &mut Map<String, Value>,
    query: &mut Query,
    network: &str,
) -> Result<(), ParserError> {
    if query
        .take("headerType")
        .is_some_and(|header| !header.is_empty() && header != "none")
    {
        return Err(UNSUPPORTED_FEATURE);
    }
    let host = query.take("host");
    let path = query.take("path");
    let service = query.take("serviceName");
    let mode = query.take("mode");
    let (key, opts) = match network {
        "tcp" => {
            if host.is_some() || path.is_some() || service.is_some() || mode.is_some() {
                return Err(UNSUPPORTED_FEATURE);
            }
            return Ok(());
        }
        "ws" | "httpupgrade" => {
            if service.is_some() || mode.is_some() {
                return Err(UNSUPPORTED_FEATURE);
            }
            let mut opts = Map::new();
            if let Some(path) = path {
                opts.insert("path".into(), Value::String(path));
            }
            if let Some(host) = host {
                opts.insert("headers".into(), json!({ "Host": host }));
            }
            if network == "httpupgrade" {
                node.insert("network".into(), json!("ws"));
                opts.insert("v2ray-http-upgrade".into(), Value::Bool(true));
            }
            ("ws-opts", opts)
        }
        "grpc" => {
            if host.is_some() || path.is_some() || mode.is_some() {
                return Err(UNSUPPORTED_FEATURE);
            }
            let mut opts = Map::new();
            if let Some(service) = service {
                opts.insert("grpc-service-name".into(), Value::String(service));
            }
            ("grpc-opts", opts)
        }
        "h2" => {
            if service.is_some() || mode.is_some() {
                return Err(UNSUPPORTED_FEATURE);
            }
            let mut opts = Map::new();
            if let Some(host) = host {
                opts.insert("host".into(), json!([host]));
            }
            if let Some(path) = path {
                opts.insert("path".into(), Value::String(path));
            }
            ("h2-opts", opts)
        }
        "http" => {
            if service.is_some() || mode.is_some() {
                return Err(UNSUPPORTED_FEATURE);
            }
            let mut opts = Map::new();
            if let Some(path) = path {
                opts.insert("path".into(), json!([path]));
            }
            if let Some(host) = host {
                opts.insert("headers".into(), json!({ "Host": [host] }));
            }
            ("http-opts", opts)
        }
        "xhttp" => {
            if service.is_some() {
                return Err(UNSUPPORTED_FEATURE);
            }
            let mut opts = Map::new();
            for (field, value) in [("path", path), ("host", host), ("mode", mode)] {
                if let Some(value) = value {
                    opts.insert(field.into(), Value::String(value));
                }
            }
            ("xhttp-opts", opts)
        }
        // The native validator decides whether this network exists for the protocol.
        _ => return Ok(()),
    };
    if !opts.is_empty() {
        node.insert(key.into(), Value::Object(opts));
    }
    Ok(())
}

fn alpn_list(value: &str) -> Result<Value, ParserError> {
    let items: Vec<Value> = value
        .split(',')
        .map(|item| Value::String(item.to_owned()))
        .collect();
    if items.iter().any(|item| item.as_str() == Some("")) {
        return Err(MALFORMED);
    }
    Ok(Value::Array(items))
}

fn flag(value: &str) -> Result<bool, ParserError> {
    match value {
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        _ => Err(MALFORMED),
    }
}

/// Strict percent-decoding: `%` must start a valid escape and the result must be UTF-8.
fn percent_decode(value: &str) -> Result<String, ParserError> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = bytes.get(index + 1..index + 3).ok_or(MALFORMED)?;
            let hex = std::str::from_utf8(hex).map_err(|_| MALFORMED)?;
            out.push(u8::from_str_radix(hex, 16).map_err(|_| MALFORMED)?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).map_err(|_| ParserError::InvalidUtf8)
}

fn parse_vmess(rest: &str) -> Result<Value, ParserError> {
    // v2rayN: base64(JSON); a trailing `#fragment` is not part of the node (name is `ps`).
    let payload = rest.split('#').next().unwrap_or(rest);
    let bytes = decode_b64_loose(payload).ok_or(MALFORMED)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| ParserError::InvalidUtf8)?;
    let value: Value = serde_json::from_str(text).map_err(|_| MALFORMED)?;
    let Value::Object(mut object) = value else {
        return Err(MALFORMED);
    };
    const KEYS: &[&str] = &[
        "v", "ps", "add", "port", "id", "aid", "scy", "net", "type", "host", "path", "tls", "sni",
        "alpn", "fp",
    ];
    if object.keys().any(|key| !KEYS.contains(&key.as_str())) {
        return Err(UNSUPPORTED_FIELD);
    }
    let text_field =
        |object: &Map<String, Value>, key: &str| -> Result<Option<String>, ParserError> {
            match object.get(key) {
                None => Ok(None),
                Some(Value::String(text)) => Ok(Some(text.clone())),
                Some(_) => Err(MALFORMED),
            }
        };
    let number_field =
        |object: &Map<String, Value>, key: &str| -> Result<Option<u64>, ParserError> {
            match object.get(key) {
                None => Ok(None),
                Some(Value::String(text)) if text.is_empty() => Ok(None),
                Some(Value::String(text)) => text.parse().map(Some).map_err(|_| MALFORMED),
                Some(Value::Number(number)) => number.as_u64().map(Some).ok_or(MALFORMED),
                Some(_) => Err(MALFORMED),
            }
        };
    let port = number_field(&object, "port")?.ok_or(MALFORMED)?;
    if !(1..=65535).contains(&port) {
        return Err(MALFORMED);
    }
    let network = text_field(&object, "net")?.unwrap_or_else(|| "tcp".into());
    let tls = match text_field(&object, "tls")?.as_deref() {
        None | Some("") | Some("none") => false,
        Some("tls") => true,
        Some(_) => return Err(UNSUPPORTED_FEATURE),
    };
    let mut node = Map::new();
    node.insert(
        "name".into(),
        json!(text_field(&object, "ps")?.unwrap_or_default()),
    );
    node.insert("type".into(), json!("vmess"));
    node.insert(
        "server".into(),
        json!(text_field(&object, "add")?.unwrap_or_default()),
    );
    node.insert("port".into(), json!(port));
    node.insert(
        "uuid".into(),
        json!(text_field(&object, "id")?.unwrap_or_default()),
    );
    node.insert(
        "cipher".into(),
        json!(
            text_field(&object, "scy")?
                .filter(|cipher| !cipher.is_empty())
                .unwrap_or_else(|| "auto".into())
        ),
    );
    node.insert(
        "alterId".into(),
        json!(number_field(&object, "aid")?.unwrap_or(0)),
    );
    node.insert("network".into(), json!(network));
    node.insert("tls".into(), Value::Bool(tls));
    // Reuse the query mapping for transport and TLS fields of the JSON form.
    let mut query = Query(BTreeMap::new());
    for (json_key, param) in [
        ("type", "headerType"),
        ("host", "host"),
        ("path", "path"),
        ("sni", "sni"),
        ("alpn", "alpn"),
        ("fp", "fp"),
    ] {
        if let Some(value) = text_field(&object, json_key)?.filter(|value| !value.is_empty()) {
            query.0.insert(param.into(), value);
        }
    }
    object.clear();
    if network == "grpc"
        && let Some(path) = query.take("path")
    {
        query.0.insert("serviceName".into(), path);
    }
    tls_params(&mut node, &mut query, "servername")?;
    transport_params(&mut node, &mut query, &network)?;
    query.finish()?;
    Ok(Value::Object(node))
}

fn parse_ss(rest: &str) -> Result<Value, ParserError> {
    let (body, name) = rest.split_once('#').unwrap_or((rest, ""));
    let name = percent_decode(name)?;
    if name.is_empty() {
        return Err(MALFORMED);
    }
    let (method, password, hostport) = if let Some((userinfo, hostport)) = body.rsplit_once('@') {
        // SIP002: userinfo is base64(method:password) or percent-encoded method:password.
        let userinfo = percent_decode(userinfo)?;
        let (method, password) = match userinfo.split_once(':') {
            Some((method, password)) => (method.to_owned(), password.to_owned()),
            None => {
                let text = decode_b64_loose(&userinfo)
                    .and_then(|bytes| String::from_utf8(bytes).ok())
                    .ok_or(MALFORMED)?;
                let (method, password) = text.split_once(':').ok_or(MALFORMED)?;
                (method.to_owned(), password.to_owned())
            }
        };
        let (hostport, query) = hostport.split_once('?').unwrap_or((hostport, ""));
        let mut query = Query::parse(query.trim_end_matches('/'))?;
        if query.take("plugin").is_some() {
            return Err(UNSUPPORTED_FEATURE);
        }
        query.finish()?;
        (method, password, hostport.trim_end_matches('/').to_owned())
    } else {
        // Legacy: base64(method:password@host:port).
        let decoded = decode_b64_loose(body).ok_or(MALFORMED)?;
        let text = String::from_utf8(decoded).map_err(|_| ParserError::InvalidUtf8)?;
        let (user, hostport) = text.rsplit_once('@').ok_or(MALFORMED)?;
        let (method, password) = user.split_once(':').ok_or(MALFORMED)?;
        (method.to_owned(), password.to_owned(), hostport.to_owned())
    };
    let (server, port) = split_host_port(&hostport)?;
    Ok(json!({
        "name": name,
        "type": "ss",
        "server": server,
        "port": port,
        "cipher": method,
        "password": password,
    }))
}

fn split_host_port(hostport: &str) -> Result<(String, u64), ParserError> {
    let (host, port) = if let Some(inner) = hostport.strip_prefix('[') {
        let (host, port) = inner.split_once("]:").ok_or(MALFORMED)?;
        (host, port)
    } else {
        let (host, port) = hostport.rsplit_once(':').ok_or(MALFORMED)?;
        if host.contains(':') {
            // Bare IPv6 is ambiguous with the port separator.
            return Err(MALFORMED);
        }
        (host, port)
    };
    let port: u64 = port.parse().map_err(|_| MALFORMED)?;
    if host.is_empty() || !(1..=65535).contains(&port) {
        return Err(MALFORMED);
    }
    Ok((host.to_owned(), port))
}

/// Base64 in either alphabet, padding optional. Whitespace is not accepted inside one value.
pub(crate) fn decode_b64_loose(text: &str) -> Option<Vec<u8>> {
    fn val(byte: u8) -> Option<u32> {
        match byte {
            b'A'..=b'Z' => Some(u32::from(byte - b'A')),
            b'a'..=b'z' => Some(u32::from(byte - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(byte - b'0') + 52),
            b'+' | b'-' => Some(62),
            b'/' | b'_' => Some(63),
            _ => None,
        }
    }
    let trimmed = text.trim_end_matches('=');
    if trimmed.is_empty() || text.len() - trimmed.len() > 2 || trimmed.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(trimmed.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for byte in trimmed.bytes() {
        acc = (acc << 6) | val(byte)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

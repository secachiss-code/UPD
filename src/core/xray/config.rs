//! Конфиг Xray из узлов в формате mihomo. Ошибка не содержит адрес, пароль или uuid.

use serde_json::{Map, Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XrayConfigError {
    Empty,
    InvalidNode,
    Unsupported(&'static str),
    InvalidPort,
}

impl std::fmt::Display for XrayConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("proxy list is empty"),
            Self::InvalidNode => f.write_str("proxy node is invalid"),
            Self::Unsupported(reason) => write!(f, "unsupported {reason}"),
            Self::InvalidPort => f.write_str("port is invalid"),
        }
    }
}

impl std::error::Error for XrayConfigError {}

pub fn outbound(node: &Value, tag: &str) -> Result<Value, XrayConfigError> {
    let object = node.as_object().ok_or(XrayConfigError::InvalidNode)?;
    if object.contains_key("dialer-proxy")
        || object.contains_key("interface-name")
        || object.contains_key("routing-mark")
    {
        return Err(XrayConfigError::Unsupported("dialer"));
    }
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or(XrayConfigError::InvalidNode)?;
    reject_unknown_fields(object, kind)?;
    let address = required_string(object, "server")?;
    let port = required_port(object)?;
    let mut settings = match kind {
        "ss" => json!({
            "servers": [{
                "address": address,
                "port": port,
                "method": required_string(object, "cipher")?,
                "password": required_string(object, "password")?,
            }]
        }),
        "trojan" => json!({
            "servers": [{
                "address": address,
                "port": port,
                "password": required_string(object, "password")?,
            }]
        }),
        "vmess" => {
            let uuid = required_string(object, "uuid")?;
            if !is_uuid(uuid) {
                return Err(XrayConfigError::InvalidNode);
            }
            let alter_id = match object.get("alterId") {
                None => 0,
                Some(Value::Number(number)) => {
                    number.as_u64().ok_or(XrayConfigError::InvalidNode)?
                }
                Some(_) => return Err(XrayConfigError::InvalidNode),
            };
            let security = object
                .get("cipher")
                .and_then(Value::as_str)
                .unwrap_or("auto");
            json!({
                "vnext": [{
                    "address": address,
                    "port": port,
                    "users": [{ "id": uuid, "alterId": alter_id, "security": security }]
                }]
            })
        }
        "vless" => {
            let uuid = required_string(object, "uuid")?;
            if !is_uuid(uuid) {
                return Err(XrayConfigError::InvalidNode);
            }
            let mut user = Map::new();
            user.insert("id".to_owned(), Value::String(uuid.to_owned()));
            user.insert("encryption".to_owned(), Value::String("none".to_owned()));
            if let Some(flow) = object.get("flow").and_then(Value::as_str) {
                if flow.is_empty() {
                    return Err(XrayConfigError::InvalidNode);
                }
                user.insert("flow".to_owned(), Value::String(flow.to_owned()));
            }
            json!({
                "vnext": [{ "address": address, "port": port, "users": [Value::Object(user)] }]
            })
        }
        "tuic" => return Err(XrayConfigError::Unsupported("tuic")),
        "hysteria2" => return Err(XrayConfigError::Unsupported("hysteria2")),
        "wireguard" => return Err(XrayConfigError::Unsupported("wireguard")),
        "http" => return Err(XrayConfigError::Unsupported("http")),
        "socks5" => return Err(XrayConfigError::Unsupported("socks5")),
        _ => return Err(XrayConfigError::Unsupported("type")),
    };
    let mut outbound = Map::new();
    outbound.insert("tag".to_owned(), Value::String(tag.to_owned()));
    outbound.insert(
        "protocol".to_owned(),
        Value::String(
            match kind {
                "ss" => "shadowsocks",
                "trojan" => "trojan",
                "vmess" => "vmess",
                "vless" => "vless",
                _ => return Err(XrayConfigError::Unsupported("type")),
            }
            .to_owned(),
        ),
    );
    if let Some(stream) = stream_settings(object, kind)? {
        outbound.insert("streamSettings".to_owned(), stream);
    }
    if let Value::Object(map) = &mut settings {
        outbound.insert("settings".to_owned(), Value::Object(std::mem::take(map)));
    }
    Ok(Value::Object(outbound))
}

pub fn generate_config(nodes: &[Value], leased_port: u16) -> Result<Vec<u8>, XrayConfigError> {
    if leased_port == 0 {
        return Err(XrayConfigError::InvalidPort);
    }
    let Some(node) = nodes.first() else {
        return Err(XrayConfigError::Empty);
    };
    let document = json!({
        "log": { "loglevel": "warning" },
        "inbounds": [{
            "tag": "cm",
            "listen": "127.0.0.1",
            "port": leased_port,
            "protocol": "mixed",
            "settings": { "udp": true }
        }],
        "outbounds": [
            outbound(node, "node")?,
            { "tag": "direct", "protocol": "freedom" },
            { "tag": "block", "protocol": "blackhole" }
        ],
        "routing": {
            "rules": [{ "type": "field", "inboundTag": ["cm"], "outboundTag": "node" }]
        }
    });
    serde_json::to_vec(&document).map_err(|_| XrayConfigError::InvalidNode)
}

/// Поля, которые конвертер переносит в конфиг Xray.
const COMMON_FIELDS: &[&str] = &[
    "name",
    "type",
    "server",
    "port",
    "network",
    "tls",
    "servername",
    "sni",
    "skip-cert-verify",
    "client-fingerprint",
    "alpn",
    "ws-opts",
    "grpc-opts",
    "reality-opts",
];
/// Подсказки mihomo про UDP и сокет. На то, с каким сервером и как шифруется соединение,
/// они не влияют, поэтому пропускаются.
const IGNORED_FIELDS: &[&str] = &[
    "udp",
    "tfo",
    "mptcp",
    "ip-version",
    "packet-encoding",
    "xudp",
];

/// Незнакомое поле — отказ: молча пропущенный `plugin` или `smux` дал бы узел, который
/// выглядит принятым, но соединяется не так, как описано в подписке.
fn reject_unknown_fields(node: &Map<String, Value>, kind: &str) -> Result<(), XrayConfigError> {
    let own: &[&str] = match kind {
        "ss" => &["cipher", "password"],
        "trojan" => &["password"],
        "vmess" => &["uuid", "alterId", "cipher"],
        "vless" => &["uuid", "flow"],
        _ => return Ok(()),
    };
    let known = |key: &str| {
        COMMON_FIELDS.contains(&key) || IGNORED_FIELDS.contains(&key) || own.contains(&key)
    };
    if !node.keys().all(|key| known(key)) {
        return Err(XrayConfigError::Unsupported("field"));
    }
    only_fields(node.get("ws-opts"), &["path", "headers"])?;
    only_fields(
        node.get("ws-opts")
            .and_then(Value::as_object)
            .and_then(|opts| opts.get("headers")),
        &["Host"],
    )?;
    only_fields(node.get("grpc-opts"), &["grpc-service-name"])?;
    only_fields(node.get("reality-opts"), &["public-key", "short-id"])
}

fn only_fields(value: Option<&Value>, allowed: &[&str]) -> Result<(), XrayConfigError> {
    let Some(value) = value else {
        return Ok(());
    };
    let object = value.as_object().ok_or(XrayConfigError::InvalidNode)?;
    if object.keys().all(|key| allowed.contains(&key.as_str())) {
        Ok(())
    } else {
        Err(XrayConfigError::Unsupported("field"))
    }
}

fn alpn(node: &Map<String, Value>) -> Result<Option<Value>, XrayConfigError> {
    let Some(value) = node.get("alpn") else {
        return Ok(None);
    };
    let items = value.as_array().ok_or(XrayConfigError::InvalidNode)?;
    if items.is_empty()
        || !items
            .iter()
            .all(|item| item.as_str().is_some_and(|text| !text.is_empty()))
    {
        return Err(XrayConfigError::InvalidNode);
    }
    Ok(Some(value.clone()))
}

fn stream_settings(
    node: &Map<String, Value>,
    kind: &str,
) -> Result<Option<Value>, XrayConfigError> {
    let network = match node.get("network") {
        None => "tcp",
        Some(Value::String(value)) => value.as_str(),
        Some(_) => return Err(XrayConfigError::InvalidNode),
    };
    if !matches!(network, "tcp" | "ws" | "grpc") {
        return Err(XrayConfigError::Unsupported("network"));
    }
    let mut stream = Map::new();
    if network != "tcp" {
        stream.insert("network".to_owned(), Value::String(network.to_owned()));
    }
    if network == "ws" {
        stream.insert("wsSettings".to_owned(), ws_settings(node)?);
    }
    if network == "grpc"
        && let Some(name) = node
            .get("grpc-opts")
            .and_then(Value::as_object)
            .and_then(|opts| opts.get("grpc-service-name"))
            .and_then(Value::as_str)
    {
        stream.insert("grpcSettings".to_owned(), json!({ "serviceName": name }));
    }
    if let Some(reality) = node.get("reality-opts").and_then(Value::as_object) {
        let fingerprint = node
            .get("client-fingerprint")
            .and_then(Value::as_str)
            .unwrap_or("chrome");
        stream.insert("security".to_owned(), Value::String("reality".to_owned()));
        stream.insert(
            "realitySettings".to_owned(),
            json!({
                "serverName": server_name(node).unwrap_or(""),
                "publicKey": required_string(reality, "public-key")?,
                "shortId": required_string(reality, "short-id")?,
                "fingerprint": fingerprint,
            }),
        );
    } else if kind == "trojan" || node.get("tls").and_then(Value::as_bool) == Some(true) {
        let mut tls = Map::new();
        if let Some(name) = server_name(node) {
            tls.insert("serverName".to_owned(), Value::String(name.to_owned()));
        }
        if node.get("skip-cert-verify").and_then(Value::as_bool) == Some(true) {
            tls.insert("allowInsecure".to_owned(), Value::Bool(true));
        }
        if let Some(fingerprint) = node.get("client-fingerprint").and_then(Value::as_str) {
            tls.insert(
                "fingerprint".to_owned(),
                Value::String(fingerprint.to_owned()),
            );
        }
        if let Some(alpn) = alpn(node)? {
            tls.insert("alpn".to_owned(), alpn);
        }
        stream.insert("security".to_owned(), Value::String("tls".to_owned()));
        stream.insert("tlsSettings".to_owned(), Value::Object(tls));
    } else if node.contains_key("alpn") {
        return Err(XrayConfigError::Unsupported("field"));
    }
    if stream.is_empty() {
        Ok(None)
    } else {
        Ok(Some(Value::Object(stream)))
    }
}

fn ws_settings(node: &Map<String, Value>) -> Result<Value, XrayConfigError> {
    let opts = node.get("ws-opts").and_then(Value::as_object);
    let mut ws = Map::new();
    if let Some(path) = opts
        .and_then(|opts| opts.get("path"))
        .and_then(Value::as_str)
    {
        ws.insert("path".to_owned(), Value::String(path.to_owned()));
    }
    if let Some(host) = opts
        .and_then(|opts| opts.get("headers"))
        .and_then(Value::as_object)
        .and_then(|headers| headers.get("Host"))
        .and_then(Value::as_str)
    {
        ws.insert("headers".to_owned(), json!({ "Host": host }));
    }
    Ok(Value::Object(ws))
}

fn server_name(node: &Map<String, Value>) -> Option<&str> {
    node.get("servername")
        .and_then(Value::as_str)
        .or_else(|| node.get("sni").and_then(Value::as_str))
        .filter(|name| !name.is_empty())
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a str, XrayConfigError> {
    match object.get(key).and_then(Value::as_str) {
        Some(text) if !text.is_empty() => Ok(text),
        _ => Err(XrayConfigError::InvalidNode),
    }
}

fn required_port(object: &Map<String, Value>) -> Result<u64, XrayConfigError> {
    let port = object
        .get("port")
        .and_then(Value::as_u64)
        .ok_or(XrayConfigError::InvalidNode)?;
    if (1..=65535).contains(&port) {
        Ok(port)
    } else {
        Err(XrayConfigError::InvalidNode)
    }
}

fn is_uuid(text: &str) -> bool {
    let mut parts = text.split('-');
    for length in [8, 4, 4, 4, 12] {
        let Some(part) = parts.next() else {
            return false;
        };
        if part.len() != length || !part.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return false;
        }
    }
    parts.next().is_none()
}

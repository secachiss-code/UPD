//! App-worker config generation.
//!
//! A worker may listen only on the port leased for it. Host routing keys are
//! rejected instead of being copied into the config.

use serde_json::{Map, Value};

const FORBIDDEN: &[&str] = &["auto-route", "auto-redirect", "dns-hijack"];

/// mihomo keys that open a listener or controller outside the leased one. A worker owns
/// exactly one leased loopback inbound; any of these is refused, never dropped.
const HOST_LISTENER_KEYS: &[&str] = &[
    "mixed-port",
    "socks-port",
    "redir-port",
    "tproxy-port",
    "external-controller",
    "external-controller-tls",
    "external-controller-unix",
    "external-controller-pipe",
    "external-ui",
    "allow-lan",
    "bind-address",
    "tun",
    "tunnels",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerError {
    Forbidden,
    ExternalListener,
    UnleasedListener,
}

impl std::fmt::Display for WorkerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Forbidden => "worker config contains a host routing key",
            Self::ExternalListener => "worker listener is not loopback",
            Self::UnleasedListener => "worker listener is not a leased port",
        })
    }
}

impl std::error::Error for WorkerError {}

/// Build a worker config that listens on `127.0.0.1` and the leased port.
/// `requested` is the caller's raw document; forbidden keys and any other
/// listener are errors, not silent omissions.
pub fn generate(requested: &Value, leased_port: u16) -> Result<String, WorkerError> {
    reject_forbidden(requested)?;
    reject_foreign_listeners(requested, leased_port)?;
    let mut root = Map::new();
    let mut inbound = Map::new();
    inbound.insert("listen".to_owned(), Value::String("127.0.0.1".to_owned()));
    inbound.insert("port".to_owned(), Value::from(leased_port));
    root.insert(
        "listeners".to_owned(),
        Value::Array(vec![Value::Object(inbound)]),
    );
    serde_json::to_string(&root).map_err(|_| WorkerError::Forbidden)
}

fn reject_forbidden(value: &Value) -> Result<(), WorkerError> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if FORBIDDEN.contains(&key.as_str()) {
                    return Err(WorkerError::Forbidden);
                }
                reject_forbidden(child)?;
            }
            Ok(())
        }
        Value::Array(items) => {
            for item in items {
                reject_forbidden(item)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn reject_foreign_listeners(value: &Value, leased_port: u16) -> Result<(), WorkerError> {
    let Some(object) = value.as_object() else {
        return Ok(());
    };
    if object.keys().any(|key| HOST_LISTENER_KEYS.contains(&key.as_str())) {
        return Err(WorkerError::ExternalListener);
    }
    // Resolver settings are allowed; a DNS *listener* is not.
    if object
        .get("dns")
        .and_then(Value::as_object)
        .is_some_and(|dns| dns.contains_key("listen"))
    {
        return Err(WorkerError::ExternalListener);
    }
    if let Some(listen) = object.get("listen")
        && !matches!(listen.as_str(), Some("127.0.0.1" | "localhost"))
    {
        return Err(WorkerError::ExternalListener);
    }
    // Any port that is not exactly the leased number (including `"7890"` as a string).
    if let Some(port) = object.get("port")
        && port.as_u64() != Some(u64::from(leased_port))
    {
        return Err(WorkerError::UnleasedListener);
    }
    if let Some(listeners) = object.get("listeners").and_then(Value::as_array) {
        for listener in listeners {
            reject_foreign_listeners(listener, leased_port)?;
        }
    }
    Ok(())
}

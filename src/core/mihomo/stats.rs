//! Traffic counters from one worker's API.
//!
//! These numbers are the core's own connection totals. They are not added to
//! host interface counters or to a TUN device counter.

use std::path::Path;
use std::time::Duration;

use super::api;
use crate::core::adapter::{CoreError, CoreStatistics};

/// `GET /connections` on `socket`. Upload and download stay separate.
pub fn read_statistics(socket: &Path, uid: u32) -> Result<CoreStatistics, CoreError> {
    let value = api::request(
        socket,
        uid,
        "GET",
        "/connections",
        None,
        Duration::from_secs(3),
    )
    .map_err(|_| CoreError::ApiUnavailable)?;
    let upload_bytes = required_u64(&value, "uploadTotal")?;
    let download_bytes = required_u64(&value, "downloadTotal")?;
    // Mihomo sends `null` after the last connection closes. That is an empty
    // list. A missing field is not zero.
    let connections = match value.get("connections") {
        Some(serde_json::Value::Array(items)) => u32::try_from(items.len()).unwrap_or(u32::MAX),
        Some(serde_json::Value::Null) => 0,
        _ => return Err(CoreError::ApiUnavailable),
    };
    Ok(CoreStatistics {
        upload_bytes,
        download_bytes,
        connections,
    })
}

fn required_u64(value: &serde_json::Value, key: &str) -> Result<u64, CoreError> {
    value
        .get(key)
        .and_then(|item| item.as_u64())
        .ok_or(CoreError::ApiUnavailable)
}

#[cfg(test)]
mod tests {
    use super::required_u64;
    use crate::core::adapter::CoreError;
    use serde_json::json;

    #[test]
    fn a_missing_counter_is_not_reported_as_zero() {
        let value = json!({"downloadTotal": 4, "connections": []});
        assert_eq!(
            required_u64(&value, "uploadTotal").unwrap_err(),
            CoreError::ApiUnavailable
        );
        assert_eq!(
            required_u64(&json!({"uploadTotal": 9}), "uploadTotal").unwrap(),
            9
        );
    }
}

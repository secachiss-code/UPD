//! HTTP/1.1 over the mihomo unix socket.
//!
//! Socket owner and `SO_PEERCRED` are checked before any request bytes are sent.
//! The host VPN and a worker share this path so the check is not copied.

use std::io::{Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::Path;
use std::time::Duration;

use serde_json::Value;

/// uid of the process on the other end of a connected unix socket.
pub fn peer_uid(stream: &std::os::unix::net::UnixStream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: cred and len describe a live ucred-sized buffer; the fd belongs to stream.
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut libc::ucred as *mut libc::c_void,
            &mut len,
        )
    };
    (result == 0 && len as usize == std::mem::size_of::<libc::ucred>()).then_some(cred.uid)
}

/// The socket and its parent directory belong to `uid`, and the directory is not group/other accessible.
pub fn check_api_socket(path: &Path, uid: u32) -> Result<(), String> {
    let bad = || {
        t!(
            "{0}: сокет API mihomo чужой или доступен не только службе",
            path.display()
        )
    };
    let meta =
        std::fs::symlink_metadata(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let parent = path.parent().ok_or_else(bad)?;
    let dir = std::fs::symlink_metadata(parent)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    if !meta.file_type().is_socket()
        || meta.uid() != uid
        || !dir.is_dir()
        || dir.uid() != uid
        || dir.mode() & 0o077 != 0
    {
        return Err(bad());
    }
    Ok(())
}

fn api_limit(path: &str) -> u64 {
    if path.starts_with("/proxies")
        || path.starts_with("/connections")
        || path.starts_with("/group")
    {
        16 << 20
    } else {
        1 << 20
    }
}

fn parse_http_response(raw: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let bad = || t!("неверный HTTP ответ").to_string();
    let head_end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(bad)?;
    let head = std::str::from_utf8(&raw[..head_end]).map_err(|_| bad())?;
    let mut lines = head.split("\r\n");
    let status: u16 = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or_else(bad)?;
    let (mut length, mut chunked) = (None, false);
    for line in lines {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "content-length" => length = Some(value.trim().parse::<usize>().map_err(|_| bad())?),
            "transfer-encoding" => chunked = value.to_ascii_lowercase().contains("chunked"),
            _ => {}
        }
    }
    let body = &raw[head_end + 4..];
    if chunked {
        let mut out = Vec::new();
        let mut pos = 0;
        loop {
            let remaining = body.get(pos..).ok_or_else(bad)?;
            let line_end = remaining
                .windows(2)
                .position(|window| window == b"\r\n")
                .ok_or_else(bad)?
                + pos;
            let size_str = std::str::from_utf8(&body[pos..line_end]).map_err(|_| bad())?;
            let size = usize::from_str_radix(size_str.split(';').next().unwrap_or("").trim(), 16)
                .map_err(|_| bad())?;
            pos = line_end + 2;
            if size == 0 {
                if body.get(pos..pos + 2) != Some(b"\r\n") {
                    return Err(bad());
                }
                return Ok((status, out));
            }
            let end = pos
                .checked_add(size)
                .filter(|end| *end <= body.len())
                .ok_or_else(bad)?;
            out.extend_from_slice(&body[pos..end]);
            if body.get(end..end + 2) != Some(b"\r\n") {
                return Err(bad());
            }
            pos = end + 2;
        }
    }
    match length {
        Some(size) if size <= body.len() => Ok((status, body[..size].to_vec())),
        Some(_) => Err(t!("ответ обрезан относительно Content-Length").into()),
        None => Ok((status, body.to_vec())),
    }
}

/// One request. `uid` is the only acceptable socket owner and peer.
pub fn request(
    socket: &Path,
    uid: u32,
    method: &str,
    path: &str,
    body: Option<Value>,
    request_timeout: Duration,
) -> Result<Value, String> {
    if !path.starts_with('/')
        || path
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte == b'\n')
    {
        return Err(t!("неверный HTTP ответ").into());
    }
    check_api_socket(socket, uid)?;
    let mut stream = std::os::unix::net::UnixStream::connect(socket)
        .map_err(|error| format!("{}: {error}", socket.display()))?;
    if peer_uid(&stream) != Some(uid) {
        return Err(t!("{0}: на сокете API не процесс службы", socket.display()));
    }
    stream
        .set_read_timeout(Some(request_timeout))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(request_timeout))
        .map_err(|error| error.to_string())?;
    let body = body.map(|value| value.to_string()).unwrap_or_default();
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: mihomo\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    if !body.is_empty() {
        req.push_str("Content-Type: application/json\r\n");
    }
    req.push_str("\r\n");
    req.push_str(&body);
    stream
        .write_all(req.as_bytes())
        .map_err(|error| error.to_string())?;
    let max = api_limit(path);
    let mut raw = Vec::new();
    (&mut stream)
        .take(max.saturating_add(1))
        .read_to_end(&mut raw)
        .map_err(|error| error.to_string())?;
    if raw.len() as u64 > max {
        return Err(t!(
            "ответ превышает лимит {}",
            crate::common::fmt_bytes(max)
        ));
    }
    let (status, body) = parse_http_response(&raw)?;
    let text = String::from_utf8_lossy(&body);
    match status {
        204 => Ok(Value::Null),
        200..=299 if text.trim().is_empty() => Ok(Value::Null),
        200..=299 => serde_json::from_str(&text).map_err(|error| error.to_string()),
        code => {
            let message = serde_json::from_str::<Value>(&text).ok().and_then(|value| {
                value
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            });
            Err(message.unwrap_or_else(|| format!("HTTP {code}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_http_response;

    #[test]
    fn truncated_content_length_is_rejected() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc";
        assert!(parse_http_response(raw).is_err());
    }

    #[test]
    fn truncated_chunked_body_is_rejected_without_panicking() {
        for raw in [
            &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1\r\na"[..],
            &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1\r\na\r\n0\r\n"[..],
        ] {
            assert!(parse_http_response(raw).is_err());
        }
    }
}

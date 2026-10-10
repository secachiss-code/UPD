//! Один кадр контроллеру и один ответ. Идентификатор — 16 hex из `/dev/urandom`.

use std::io::{BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use super::codec::read_frame;
use super::protocol::{CONTROL_VERSION, Op, Reply};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientError {
    Unavailable,
    BadReply,
}

pub fn call(socket: &Path, op: Op) -> Result<Reply, ClientError> {
    let mut stream = UnixStream::connect(socket).map_err(|_| ClientError::Unavailable)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|_| ClientError::Unavailable)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(30)))
        .map_err(|_| ClientError::Unavailable)?;
    let request = super::protocol::Request {
        v: CONTROL_VERSION,
        id: random_id()?,
        op,
    };
    let bytes = serde_json::to_vec(&request).map_err(|_| ClientError::BadReply)?;
    stream
        .write_all(&bytes)
        .and_then(|_| stream.write_all(b"\n"))
        .map_err(|_| ClientError::Unavailable)?;
    let mut reader = BufReader::new(stream);
    let frame = match read_frame(&mut reader) {
        Ok(Some(frame)) => frame,
        _ => return Err(ClientError::BadReply),
    };
    if frame.len() > super::protocol::MAX_FRAME_BYTES {
        return Err(ClientError::BadReply);
    }
    serde_json::from_slice(&frame).map_err(|_| ClientError::BadReply)
}

/// The on-wire request is one JSON line. Exposed so a test can read the frame shape.
pub fn frame_of(op: Op) -> Result<Vec<u8>, ClientError> {
    let request = super::protocol::Request {
        v: CONTROL_VERSION,
        id: random_id()?,
        op,
    };
    let mut bytes = serde_json::to_vec(&request).map_err(|_| ClientError::BadReply)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn random_id() -> Result<String, ClientError> {
    let mut bytes = [0u8; 8];
    let mut file = std::fs::File::open("/dev/urandom").map_err(|_| ClientError::Unavailable)?;
    file.read_exact(&mut bytes)
        .map_err(|_| ClientError::Unavailable)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

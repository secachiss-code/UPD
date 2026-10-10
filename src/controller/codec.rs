//! Жёсткий кодек кадра: не больше 64 КиБ и только точная схема версии 1.

use std::io::{BufRead, ErrorKind};

use sha2::{Digest, Sha256};

use super::protocol::{CONTROL_VERSION, ControlError, MAX_FRAME_BYTES, Request};
use crate::core::instance::InstanceId;

/// Сколько байт слишком длинного кадра вычитывается в поисках перевода строки. Дальше
/// соединение не слушают: иначе поток без `\n` держал бы его бесконечно.
const MAX_SKIP_BYTES: usize = 4 * MAX_FRAME_BYTES;

/// Читает один кадр до `\n`. Длиннее предела — `TooLarge`; хвост до перевода строки
/// вычитывается, но не дальше [`MAX_SKIP_BYTES`].
pub fn read_frame(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>, ControlError> {
    let mut buf = Vec::new();
    let mut oversized = false;
    let mut skipped = 0usize;
    loop {
        if skipped > MAX_SKIP_BYTES {
            return Err(ControlError::TooLarge);
        }
        let available = reader.fill_buf().map_err(io_frame)?;
        if available.is_empty() {
            if buf.is_empty() && !oversized {
                return Ok(None);
            }
            return Err(if oversized {
                ControlError::TooLarge
            } else {
                ControlError::BadFrame
            });
        }
        if let Some(pos) = available.iter().position(|byte| *byte == b'\n') {
            if !oversized {
                if buf.len().saturating_add(pos) > MAX_FRAME_BYTES {
                    oversized = true;
                    buf.clear();
                } else {
                    buf.extend_from_slice(&available[..pos]);
                }
            }
            reader.consume(pos + 1);
            return if oversized {
                Err(ControlError::TooLarge)
            } else {
                Ok(Some(buf))
            };
        }
        if oversized {
            let skip = available.len();
            reader.consume(skip);
            skipped = skipped.saturating_add(skip);
            continue;
        }
        let room = MAX_FRAME_BYTES.saturating_add(1).saturating_sub(buf.len());
        if available.len() > room {
            oversized = true;
            buf.clear();
            let skip = available.len();
            reader.consume(skip);
        } else {
            buf.extend_from_slice(available);
            let skip = available.len();
            reader.consume(skip);
            if buf.len() > MAX_FRAME_BYTES {
                oversized = true;
                buf.clear();
            }
        }
    }
}

pub fn decode(frame: &[u8]) -> Result<Request, ControlError> {
    if frame.len() > MAX_FRAME_BYTES {
        return Err(ControlError::TooLarge);
    }
    let text = std::str::from_utf8(frame).map_err(|_| ControlError::BadFrame)?;
    let request: Request = serde_json::from_str(text).map_err(|_| ControlError::BadFrame)?;
    if request.v != CONTROL_VERSION {
        return Err(ControlError::UnsupportedVersion);
    }
    if !valid_id(&request.id) {
        return Err(ControlError::BadId);
    }
    if let Some(instance) = request.op.instance()
        && InstanceId::new(instance).is_err()
    {
        return Err(ControlError::BadInstance);
    }
    match &request.op {
        super::protocol::Op::WorkerReload {
            generation,
            next_generation,
            ..
        } if *next_generation <= *generation => {
            return Err(ControlError::BadArgument);
        }
        super::protocol::Op::AppLaunch {
            program, args, env, ..
        } => {
            check_launch(program, args)?;
            check_session_env(env)?;
        }
        _ => {}
    }
    Ok(request)
}

pub fn encode(reply: &super::protocol::Reply) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(reply).unwrap_or_else(|_| {
        serde_json::to_vec(&super::protocol::Reply::error("", ControlError::Failed)).unwrap_or_else(
            |_| br#"{"v":1,"id":"","ok":false,"code":"failed","data":null}"#.to_vec(),
        )
    });
    bytes.retain(|byte| *byte != b'\n');
    bytes.push(b'\n');
    bytes
}

/// sha256 (hex) канонического JSON поля `op`. От `id` не зависит.
pub fn request_digest(request: &Request) -> String {
    let bytes = serde_json::to_vec(&request.op).unwrap_or_default();
    hex(Sha256::digest(bytes))
}

fn valid_id(id: &str) -> bool {
    (8..=64).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn check_launch(program: &str, args: &[String]) -> Result<(), ControlError> {
    if !program.starts_with('/') || program.as_bytes().contains(&0) || has_dot_dot(program) {
        return Err(ControlError::BadArgument);
    }
    if args.len() > 64 {
        return Err(ControlError::BadArgument);
    }
    if args
        .iter()
        .any(|arg| arg.len() > 4096 || arg.as_bytes().contains(&0))
    {
        return Err(ControlError::BadArgument);
    }
    Ok(())
}

fn check_session_env(env: &std::collections::BTreeMap<String, String>) -> Result<(), ControlError> {
    let ok = env.iter().all(|(key, value)| {
        super::protocol::SESSION_ENV_KEYS.contains(&key.as_str())
            && value.len() <= super::protocol::MAX_ENV_VALUE_BYTES
            && !value.as_bytes().contains(&0)
    });
    if ok {
        Ok(())
    } else {
        Err(ControlError::BadArgument)
    }
}

fn has_dot_dot(path: &str) -> bool {
    path.split('/').any(|segment| segment == "..")
}

fn io_frame(error: std::io::Error) -> ControlError {
    if error.kind() == ErrorKind::TimedOut {
        ControlError::Timeout
    } else {
        ControlError::BadFrame
    }
}

fn hex(bytes: impl AsRef<[u8]>) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0xf) as usize] as char);
    }
    out
}

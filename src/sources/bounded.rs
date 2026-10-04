//! Bounded private JSON encoding shared by source artifacts and definition hashes.

use serde::Serialize;
use std::io::{self, Write};

/// Safe classifications; serde errors can contain material and never escape here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BoundedJsonError {
    Limit,
    Encoding,
}

pub(crate) fn encode_json_bounded<T: Serialize>(
    value: &T,
    limit: usize,
) -> Result<Vec<u8>, BoundedJsonError> {
    let mut writer = BoundedWriter {
        bytes: Vec::with_capacity(limit.min(4096)),
        limit,
        limit_hit: false,
    };
    let result = serde_json::to_writer(&mut writer, value);
    if writer.limit_hit {
        // Even a custom Serialize implementation that catches the writer error
        // cannot turn a truncated artifact into a successful encoding.
        Err(BoundedJsonError::Limit)
    } else {
        result.map_err(|_| BoundedJsonError::Encoding)?;
        Ok(writer.bytes)
    }
}

struct BoundedWriter {
    bytes: Vec<u8>,
    limit: usize,
    limit_hit: bool,
}

impl Write for BoundedWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.len() > self.limit.saturating_sub(self.bytes.len()) {
            self.limit_hit = true;
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "private JSON size limit exceeded",
            ));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

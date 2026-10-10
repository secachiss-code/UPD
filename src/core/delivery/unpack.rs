//! Распаковка gzip и одного файла из zip. Предел читается через `take`, без загрузки бомбы.

use std::io::{Cursor, Read};

use flate2::Crc;
use flate2::read::{DeflateDecoder, GzDecoder};

use super::pin::DeliveryError;

pub fn unpack_gzip(bytes: &[u8], max: u64) -> Result<Vec<u8>, DeliveryError> {
    let limit = max.checked_add(1).ok_or(DeliveryError::TooLarge)?;
    let mut output = Vec::new();
    GzDecoder::new(Cursor::new(bytes))
        .take(limit)
        .read_to_end(&mut output)
        .map_err(|_| DeliveryError::BadArchive)?;
    if output.len() as u64 > max {
        return Err(DeliveryError::TooLarge);
    }
    Ok(output)
}

pub fn unpack_zip_member(bytes: &[u8], member: &str, max: u64) -> Result<Vec<u8>, DeliveryError> {
    if member.is_empty()
        || member
            .split('/')
            .any(|segment| segment.is_empty() || segment == ".." || segment == ".")
        || member.contains('/')
    {
        return Err(DeliveryError::MemberMissing);
    }
    let eocd = find_eocd(bytes)?;
    let disk = read_u16(bytes, eocd + 4)?;
    let cd_disk = read_u16(bytes, eocd + 6)?;
    let entries = read_u16(bytes, eocd + 10)? as usize;
    let cd_size = read_u32(bytes, eocd + 12)? as usize;
    let cd_offset = read_u32(bytes, eocd + 16)? as usize;
    if disk != 0 || cd_disk != 0 {
        return Err(DeliveryError::BadArchive);
    }
    if entries == 0xffff || cd_size == 0xffff_ffff || cd_offset == 0xffff_ffff {
        return Err(DeliveryError::BadArchive);
    }
    if entries > 4096 {
        return Err(DeliveryError::BadArchive);
    }
    let cd_end = cd_offset
        .checked_add(cd_size)
        .ok_or(DeliveryError::BadArchive)?;
    if cd_end > bytes.len() {
        return Err(DeliveryError::BadArchive);
    }
    let mut cursor = cd_offset;
    let mut found = None;
    for _ in 0..entries {
        if cursor.saturating_add(46) > cd_end
            || bytes.get(cursor..cursor + 4) != Some(b"PK\x01\x02")
        {
            return Err(DeliveryError::BadArchive);
        }
        let flags = read_u16(bytes, cursor + 8)?;
        let method = read_u16(bytes, cursor + 10)?;
        let crc = read_u32(bytes, cursor + 16)?;
        let compressed = read_u32(bytes, cursor + 20)? as usize;
        let uncompressed = read_u32(bytes, cursor + 24)? as u64;
        let name_len = read_u16(bytes, cursor + 28)? as usize;
        let extra_len = read_u16(bytes, cursor + 30)? as usize;
        let comment_len = read_u16(bytes, cursor + 32)? as usize;
        let local_offset = read_u32(bytes, cursor + 42)? as usize;
        if compressed == 0xffff_ffff || uncompressed == 0xffff_ffff || local_offset == 0xffff_ffff {
            return Err(DeliveryError::BadArchive);
        }
        let name_at = cursor + 46;
        let name_end = name_at
            .checked_add(name_len)
            .ok_or(DeliveryError::BadArchive)?;
        let next = name_end
            .checked_add(extra_len)
            .and_then(|value| value.checked_add(comment_len))
            .ok_or(DeliveryError::BadArchive)?;
        if next > cd_end {
            return Err(DeliveryError::BadArchive);
        }
        let name = std::str::from_utf8(
            bytes
                .get(name_at..name_end)
                .ok_or(DeliveryError::BadArchive)?,
        )
        .unwrap_or("");
        if name == member {
            found = Some(Member {
                flags,
                method,
                crc,
                compressed,
                uncompressed,
                local_offset,
            });
        }
        cursor = next;
    }
    let Some(member) = found else {
        return Err(DeliveryError::MemberMissing);
    };
    if member.flags & 1 != 0 || (member.method != 0 && member.method != 8) {
        return Err(DeliveryError::BadArchive);
    }
    if member.uncompressed > max {
        return Err(DeliveryError::TooLarge);
    }
    let data = local_data(bytes, member.local_offset, member.compressed)?;
    let output = match member.method {
        0 => data.to_vec(),
        _ => inflate(data, max)?,
    };
    if output.len() as u64 != member.uncompressed {
        return Err(DeliveryError::BadArchive);
    }
    let mut crc = Crc::new();
    crc.update(&output);
    if crc.sum() != member.crc {
        return Err(DeliveryError::BadArchive);
    }
    Ok(output)
}

struct Member {
    flags: u16,
    method: u16,
    crc: u32,
    compressed: usize,
    uncompressed: u64,
    local_offset: usize,
}

fn inflate(data: &[u8], max: u64) -> Result<Vec<u8>, DeliveryError> {
    let limit = max.checked_add(1).ok_or(DeliveryError::TooLarge)?;
    let mut output = Vec::new();
    DeflateDecoder::new(Cursor::new(data))
        .take(limit)
        .read_to_end(&mut output)
        .map_err(|_| DeliveryError::BadArchive)?;
    if output.len() as u64 > max {
        return Err(DeliveryError::TooLarge);
    }
    Ok(output)
}

fn local_data(bytes: &[u8], offset: usize, compressed: usize) -> Result<&[u8], DeliveryError> {
    if bytes.get(offset..offset + 4) != Some(b"PK\x03\x04") {
        return Err(DeliveryError::BadArchive);
    }
    let name_len = read_u16(bytes, offset + 26)? as usize;
    let extra_len = read_u16(bytes, offset + 28)? as usize;
    let start = offset
        .checked_add(30)
        .and_then(|value| value.checked_add(name_len))
        .and_then(|value| value.checked_add(extra_len))
        .ok_or(DeliveryError::BadArchive)?;
    let end = start
        .checked_add(compressed)
        .ok_or(DeliveryError::BadArchive)?;
    bytes.get(start..end).ok_or(DeliveryError::BadArchive)
}

fn find_eocd(bytes: &[u8]) -> Result<usize, DeliveryError> {
    if bytes.len() < 22 {
        return Err(DeliveryError::BadArchive);
    }
    let start = bytes.len().saturating_sub(65_557);
    let mut index = bytes.len() - 22;
    loop {
        if bytes.get(index..index + 4) == Some(b"PK\x05\x06") {
            let comment = read_u16(bytes, index + 20)? as usize;
            if index + 22 + comment == bytes.len() {
                return Ok(index);
            }
        }
        if index == start {
            break;
        }
        index -= 1;
    }
    Err(DeliveryError::BadArchive)
}

fn read_u16(bytes: &[u8], at: usize) -> Result<u16, DeliveryError> {
    let slice = bytes.get(at..at + 2).ok_or(DeliveryError::BadArchive)?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32, DeliveryError> {
    let slice = bytes.get(at..at + 4).ok_or(DeliveryError::BadArchive)?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

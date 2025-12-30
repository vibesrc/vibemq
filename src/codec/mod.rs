//! MQTT Packet Codec
//!
//! Provides encoding and decoding for MQTT v3.1.1 and v5.0 packets
//! in a unified manner.

mod cached;
mod decode;
mod encode;

#[cfg(test)]
mod tests;

pub use cached::{CachedPublish, PublishCache, RawPublish};
pub use decode::Decoder;
pub use encode::Encoder;

use crate::protocol::{DecodeError, EncodeError};
use bytes::{BufMut, BytesMut};

/// Maximum remaining length (268,435,455 bytes = ~256 MB)
pub const MAX_REMAINING_LENGTH: usize = 268_435_455;

/// Maximum packet size (configurable, but capped at MAX_REMAINING_LENGTH + 5)
pub const DEFAULT_MAX_PACKET_SIZE: usize = 1024 * 1024; // 1 MB default

/// Read a Variable Byte Integer from buffer
/// Returns (value, bytes_consumed) or error
#[inline]
pub fn read_variable_int(buf: &[u8]) -> Result<(u32, usize), DecodeError> {
    let mut multiplier: u32 = 1;
    let mut value: u32 = 0;
    let mut pos = 0;

    loop {
        if pos >= buf.len() {
            return Err(DecodeError::InsufficientData);
        }
        if pos >= 4 {
            return Err(DecodeError::InvalidRemainingLength);
        }

        let byte = buf[pos];
        value += ((byte & 0x7F) as u32) * multiplier;
        pos += 1;

        if (byte & 0x80) == 0 {
            break;
        }

        multiplier *= 128;
    }

    Ok((value, pos))
}

/// Write a Variable Byte Integer to buffer
/// Returns bytes written
#[inline]
pub fn write_variable_int(buf: &mut BytesMut, mut value: u32) -> Result<usize, EncodeError> {
    if value > MAX_REMAINING_LENGTH as u32 {
        return Err(EncodeError::PacketTooLarge);
    }

    let mut count = 0;
    loop {
        let mut byte = (value % 128) as u8;
        value /= 128;
        if value > 0 {
            byte |= 0x80;
        }
        buf.put_u8(byte);
        count += 1;
        if value == 0 {
            break;
        }
    }
    Ok(count)
}

/// Calculate the number of bytes needed to encode a Variable Byte Integer
#[inline]
pub fn variable_int_len(value: u32) -> usize {
    if value < 128 {
        1
    } else if value < 16_384 {
        2
    } else if value < 2_097_152 {
        3
    } else {
        4
    }
}

/// Read a Two Byte Integer (u16 big-endian)
#[inline]
pub fn read_u16(buf: &[u8]) -> Result<u16, DecodeError> {
    if buf.len() < 2 {
        return Err(DecodeError::InsufficientData);
    }
    Ok(u16::from_be_bytes([buf[0], buf[1]]))
}

/// Read a Four Byte Integer (u32 big-endian)
#[inline]
pub fn read_u32(buf: &[u8]) -> Result<u32, DecodeError> {
    if buf.len() < 4 {
        return Err(DecodeError::InsufficientData);
    }
    Ok(u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]))
}

/// Read a UTF-8 encoded string
/// Returns (string, bytes_consumed) or error
#[inline]
pub fn read_string(buf: &[u8]) -> Result<(&str, usize), DecodeError> {
    if buf.len() < 2 {
        return Err(DecodeError::InsufficientData);
    }

    let len = u16::from_be_bytes([buf[0], buf[1]]) as usize;
    let total_len = 2 + len;

    if buf.len() < total_len {
        return Err(DecodeError::InsufficientData);
    }

    let s = std::str::from_utf8(&buf[2..total_len]).map_err(|_| DecodeError::InvalidUtf8)?;

    // Validate no null characters (per MQTT spec)
    if s.contains('\0') {
        return Err(DecodeError::MalformedPacket(
            "string contains null character",
        ));
    }

    // Note: UTF-8 naturally rejects surrogate pairs (U+D800 to U+DFFF),
    // so from_utf8 above already handles that case.

    Ok((s, total_len))
}

/// Read binary data
/// Returns (data, bytes_consumed) or error
#[inline]
pub fn read_binary(buf: &[u8]) -> Result<(&[u8], usize), DecodeError> {
    if buf.len() < 2 {
        return Err(DecodeError::InsufficientData);
    }

    let len = u16::from_be_bytes([buf[0], buf[1]]) as usize;
    let total_len = 2 + len;

    if buf.len() < total_len {
        return Err(DecodeError::InsufficientData);
    }

    Ok((&buf[2..total_len], total_len))
}

/// Write a UTF-8 encoded string
#[inline]
pub fn write_string(buf: &mut BytesMut, s: &str) -> Result<(), EncodeError> {
    let len = s.len();
    if len > 65535 {
        return Err(EncodeError::StringTooLong);
    }
    buf.put_u16(len as u16);
    buf.put_slice(s.as_bytes());
    Ok(())
}

/// Write binary data
#[inline]
pub fn write_binary(buf: &mut BytesMut, data: &[u8]) -> Result<(), EncodeError> {
    let len = data.len();
    if len > 65535 {
        return Err(EncodeError::StringTooLong);
    }
    buf.put_u16(len as u16);
    buf.put_slice(data);
    Ok(())
}

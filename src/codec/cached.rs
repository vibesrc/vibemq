//! Pre-serialized PUBLISH packets for efficient fan-out.
//!
//! When routing a message to many subscribers, encoding the same packet
//! thousands of times is wasteful. This module provides:
//!
//! - `CachedPublish`: Pre-encoded PUBLISH with per-subscriber patching
//! - `RawPublish`: Zero-copy wrapper around incoming wire bytes
//!
//! # Usage
//!
//! ```ignore
//! // For re-encoded packets:
//! let cached = CachedPublish::new(&publish, protocol_version);
//!
//! // For zero-copy from incoming wire bytes:
//! let raw = RawPublish::from_wire(raw_bytes, protocol_version)?;
//!
//! for subscriber in subscribers {
//!     // Write to subscriber's buffer with their specific packet_id
//!     cached.write_to(buf, subscriber.packet_id, subscriber.qos, subscriber.retain);
//! }
//! ```

use bytes::{BufMut, Bytes, BytesMut};

use super::read_variable_int;

use super::{variable_int_len, write_string, write_variable_int};
use crate::protocol::{EncodeError, ProtocolVersion, Publish, QoS};

/// A pre-serialized PUBLISH packet that can be efficiently written with
/// per-subscriber modifications (packet_id, qos, retain, dup).
#[derive(Debug, Clone)]
pub struct CachedPublish {
    /// The protocol version this was encoded for
    protocol_version: ProtocolVersion,

    /// Pre-encoded bytes (everything except modifiable fields)
    /// For QoS 0: complete packet with placeholder first byte
    /// For QoS 1/2: packet with placeholder first byte and packet_id
    bytes: Bytes,

    /// Offset of the first byte (always 0, but stored for clarity)
    first_byte_offset: usize,

    /// Offset where packet_id should be written (None for QoS 0 source)
    /// This is relative to the start of the buffer
    packet_id_offset: Option<usize>,
}

impl CachedPublish {
    /// Create a new cached publish from a Publish packet.
    ///
    /// The packet is encoded once. Subsequent writes use memcpy with
    /// in-place modification of first_byte and packet_id.
    pub fn new(publish: &Publish, protocol_version: ProtocolVersion) -> Result<Self, EncodeError> {
        let is_v5 = protocol_version == ProtocolVersion::V5;

        // Calculate remaining length (same as Encoder::encode_publish)
        let mut remaining_length = 2 + publish.topic.len(); // topic length prefix + topic

        // Always reserve space for packet_id - we might downgrade QoS
        // but subscribers might have higher QoS subscriptions
        remaining_length += 2; // packet identifier (always reserve)

        if is_v5 {
            let props_len = publish.properties.encoded_size();
            remaining_length += variable_int_len(props_len as u32) + props_len;
        }

        remaining_length += publish.payload.len();

        // Calculate remaining_length encoding size
        let remaining_length_size = variable_int_len(remaining_length as u32);

        // Pre-allocate buffer
        let total_size = 1 + remaining_length_size + remaining_length;
        let mut buf = BytesMut::with_capacity(total_size);

        // First byte (placeholder - will be patched per-subscriber)
        let first_byte = Self::build_first_byte(publish.dup, publish.qos, publish.retain);
        buf.put_u8(first_byte);

        // Remaining length
        write_variable_int(&mut buf, remaining_length as u32)?;

        // Topic name
        write_string(&mut buf, &publish.topic)?;

        // Packet identifier offset (right after topic)
        let packet_id_offset = buf.len();

        // Write placeholder packet_id (will be patched per-subscriber)
        buf.put_u16(publish.packet_id.unwrap_or(0));

        // Properties (v5.0 only)
        if is_v5 {
            publish.properties.encode(&mut buf)?;
        }

        // Payload
        buf.put_slice(&publish.payload);

        Ok(Self {
            protocol_version,
            bytes: buf.freeze(),
            first_byte_offset: 0,
            packet_id_offset: Some(packet_id_offset),
        })
    }

    /// Create a cached publish for QoS 0 (no packet_id in wire format).
    ///
    /// This is more efficient when the source is QoS 0 and all subscribers
    /// are also QoS 0.
    #[allow(dead_code)] // May be used for QoS 0 optimization in the future
    pub fn new_qos0(publish: &Publish, protocol_version: ProtocolVersion) -> Result<Self, EncodeError> {
        let is_v5 = protocol_version == ProtocolVersion::V5;

        // QoS 0 has no packet_id
        let mut remaining_length = 2 + publish.topic.len();

        if is_v5 {
            let props_len = publish.properties.encoded_size();
            remaining_length += variable_int_len(props_len as u32) + props_len;
        }

        remaining_length += publish.payload.len();

        let remaining_length_size = variable_int_len(remaining_length as u32);

        let total_size = 1 + remaining_length_size + remaining_length;
        let mut buf = BytesMut::with_capacity(total_size);

        // First byte
        let first_byte = Self::build_first_byte(false, QoS::AtMostOnce, publish.retain);
        buf.put_u8(first_byte);

        // Remaining length
        write_variable_int(&mut buf, remaining_length as u32)?;

        // Topic name
        write_string(&mut buf, &publish.topic)?;

        // Properties (v5.0 only)
        if is_v5 {
            publish.properties.encode(&mut buf)?;
        }

        // Payload
        buf.put_slice(&publish.payload);

        Ok(Self {
            protocol_version,
            bytes: buf.freeze(),
            first_byte_offset: 0,
            packet_id_offset: None,
        })
    }

    /// Write the cached packet to a buffer with subscriber-specific values.
    ///
    /// This is the hot path - optimized for minimal work:
    /// 1. Copy all bytes
    /// 2. Patch first byte (dup, qos, retain)
    /// 3. Patch packet_id (if QoS > 0)
    #[inline]
    pub fn write_to(
        &self,
        buf: &mut BytesMut,
        packet_id: Option<u16>,
        qos: QoS,
        retain: bool,
        dup: bool,
    ) {
        let start = buf.len();

        // Fast path: copy all bytes at once
        buf.extend_from_slice(&self.bytes);

        // Patch first byte
        let first_byte = Self::build_first_byte(dup, qos, retain);
        buf[start + self.first_byte_offset] = first_byte;

        // Patch packet_id if needed
        if let (Some(offset), Some(pid)) = (self.packet_id_offset, packet_id) {
            let id_bytes = pid.to_be_bytes();
            buf[start + offset] = id_bytes[0];
            buf[start + offset + 1] = id_bytes[1];
        }
    }

    /// Build the PUBLISH first byte from flags.
    #[inline]
    fn build_first_byte(dup: bool, qos: QoS, retain: bool) -> u8 {
        let mut byte: u8 = 0x30; // PUBLISH type (0011)
        if dup {
            byte |= 0x08;
        }
        byte |= (qos as u8) << 1;
        if retain {
            byte |= 0x01;
        }
        byte
    }

    /// Get the protocol version this was cached for.
    pub fn protocol_version(&self) -> ProtocolVersion {
        self.protocol_version
    }

    /// Get the size of the cached packet in bytes.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Check if this cached packet is empty (should never be true).
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// Zero-copy wrapper around raw incoming PUBLISH wire bytes.
///
/// Instead of decoding into a Publish struct and re-encoding for fan-out,
/// this captures the raw wire bytes and patches only the necessary fields
/// (first_byte for QoS/retain/dup, packet_id) when writing to subscribers.
///
/// This eliminates all allocations in the fan-out path.
#[derive(Debug, Clone)]
pub struct RawPublish {
    /// The original wire bytes (shared via Bytes, no copy on clone)
    bytes: Bytes,

    /// Protocol version
    protocol_version: ProtocolVersion,

    /// Offset of the first byte (for patching QoS/DUP/RETAIN)
    first_byte_offset: usize,

    /// Offset where packet_id is (for QoS 1/2 packets) or should be inserted
    packet_id_offset: usize,

    /// Whether this packet has a packet_id in the wire format
    has_packet_id: bool,

    /// Original QoS from the wire
    original_qos: QoS,
}

impl RawPublish {
    /// Create a RawPublish from incoming wire bytes.
    ///
    /// Parses just enough to find the offsets, no allocations.
    /// The bytes should be a complete PUBLISH packet including fixed header.
    pub fn from_wire(bytes: Bytes, protocol_version: ProtocolVersion) -> Result<Self, super::super::protocol::DecodeError> {
        use crate::protocol::DecodeError;

        if bytes.is_empty() {
            return Err(DecodeError::InsufficientData);
        }

        let first_byte = bytes[0];
        let packet_type = first_byte >> 4;
        if packet_type != 3 {
            return Err(DecodeError::MalformedPacket("not a PUBLISH packet"));
        }

        let qos_bits = (first_byte >> 1) & 0x03;
        let qos = QoS::from_u8(qos_bits).ok_or(DecodeError::InvalidQoS(qos_bits))?;

        // Parse remaining length
        let (remaining_length, remaining_len_bytes) = read_variable_int(&bytes[1..])?;
        let header_size = 1 + remaining_len_bytes;

        // Topic length is at start of variable header
        if bytes.len() < header_size + 2 {
            return Err(DecodeError::InsufficientData);
        }
        let topic_len = u16::from_be_bytes([bytes[header_size], bytes[header_size + 1]]) as usize;

        // Packet ID offset is right after topic (if QoS > 0)
        let packet_id_offset = header_size + 2 + topic_len;

        // Validate we have enough bytes
        let expected_len = header_size + remaining_length as usize;
        if bytes.len() < expected_len {
            return Err(DecodeError::InsufficientData);
        }

        Ok(Self {
            bytes,
            protocol_version,
            first_byte_offset: 0,
            packet_id_offset,
            has_packet_id: qos != QoS::AtMostOnce,
            original_qos: qos,
        })
    }

    /// Write to buffer with per-subscriber patching.
    ///
    /// This is the hot path - optimized for minimal work:
    /// 1. Copy all bytes at once
    /// 2. Patch first byte (dup, qos, retain)
    /// 3. Patch packet_id (if QoS > 0)
    #[inline]
    pub fn write_to(
        &self,
        buf: &mut BytesMut,
        packet_id: Option<u16>,
        qos: QoS,
        retain: bool,
        dup: bool,
    ) {
        let start = buf.len();

        // Fast path: copy all bytes at once
        buf.extend_from_slice(&self.bytes);

        // Patch first byte
        let first_byte = CachedPublish::build_first_byte(dup, qos, retain);
        buf[start + self.first_byte_offset] = first_byte;

        // Patch packet_id if needed
        if let Some(pid) = packet_id {
            if self.has_packet_id {
                // Packet already has space for packet_id, just overwrite
                let id_bytes = pid.to_be_bytes();
                buf[start + self.packet_id_offset] = id_bytes[0];
                buf[start + self.packet_id_offset + 1] = id_bytes[1];
            }
            // If original was QoS 0 but subscriber wants QoS 1/2, we can't patch
            // (would need to insert bytes). Caller should use CachedPublish instead.
        }
    }

    /// Check if this raw packet can be used for the given QoS.
    ///
    /// QoS upgrade (0 -> 1/2) is not supported with RawPublish since it requires
    /// inserting packet_id bytes. Use CachedPublish for that case.
    #[inline]
    pub fn supports_qos(&self, target_qos: QoS) -> bool {
        // Can always downgrade or match QoS
        // Can only upgrade if we have packet_id space (original QoS > 0)
        if target_qos == QoS::AtMostOnce {
            true // Always can downgrade to QoS 0
        } else {
            self.has_packet_id // Need packet_id space for QoS 1/2
        }
    }

    /// Get the protocol version.
    pub fn protocol_version(&self) -> ProtocolVersion {
        self.protocol_version
    }

    /// Get the size of the raw packet.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Get the original QoS from wire.
    pub fn original_qos(&self) -> QoS {
        self.original_qos
    }
}

/// Cache for pre-serialized publish packets.
///
/// Holds up to 4 variants per publish:
/// - v3.1.1 encoding (with packet_id space for QoS 1/2)
/// - v3.1.1 encoding for QoS 0 (no packet_id)
/// - v5.0 encoding (with packet_id space for QoS 1/2)
/// - v5.0 encoding for QoS 0 (no packet_id)
///
/// Uses Arc internally so callers can cheaply clone the cached packet
/// (just atomic increment) instead of allocating per subscriber.
#[derive(Debug, Default)]
pub struct PublishCache {
    v311: Option<std::sync::Arc<CachedPublish>>,
    v311_qos0: Option<std::sync::Arc<CachedPublish>>,
    v5: Option<std::sync::Arc<CachedPublish>>,
    v5_qos0: Option<std::sync::Arc<CachedPublish>>,
}

impl PublishCache {
    /// Create a new empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get or create a cached publish for QoS 1/2 (includes packet_id space).
    /// Returns an Arc clone (just atomic increment, no allocation).
    pub fn get_or_create(
        &mut self,
        publish: &Publish,
        protocol_version: ProtocolVersion,
    ) -> Result<std::sync::Arc<CachedPublish>, EncodeError> {
        let slot = match protocol_version {
            ProtocolVersion::V311 => &mut self.v311,
            ProtocolVersion::V5 => &mut self.v5,
        };

        if slot.is_none() {
            *slot = Some(std::sync::Arc::new(CachedPublish::new(publish, protocol_version)?));
        }

        Ok(slot.as_ref().unwrap().clone())
    }

    /// Get or create a cached publish for QoS 0 (no packet_id in wire format).
    /// Returns an Arc clone (just atomic increment, no allocation).
    pub fn get_or_create_qos0(
        &mut self,
        publish: &Publish,
        protocol_version: ProtocolVersion,
    ) -> Result<std::sync::Arc<CachedPublish>, EncodeError> {
        let slot = match protocol_version {
            ProtocolVersion::V311 => &mut self.v311_qos0,
            ProtocolVersion::V5 => &mut self.v5_qos0,
        };

        if slot.is_none() {
            *slot = Some(std::sync::Arc::new(CachedPublish::new_qos0(publish, protocol_version)?));
        }

        Ok(slot.as_ref().unwrap().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::Decoder;
    use crate::protocol::Properties;
    use std::sync::Arc;

    #[test]
    fn test_cached_publish_qos1_roundtrip() {
        let publish = Publish {
            dup: false,
            qos: QoS::AtLeastOnce,
            retain: false,
            topic: Arc::from("test/topic"),
            packet_id: Some(123),
            payload: bytes::Bytes::from("hello world"),
            properties: Properties::default(),
        };

        let cached = CachedPublish::new(&publish, ProtocolVersion::V311).unwrap();

        // Write with same packet_id
        let mut buf = BytesMut::new();
        cached.write_to(&mut buf, Some(123), QoS::AtLeastOnce, false, false);

        // Decode and verify
        let mut decoder = Decoder::new();
        decoder.set_protocol_version(ProtocolVersion::V311);
        let (decoded, _) = decoder.decode(&mut buf).unwrap().unwrap();

        if let crate::protocol::Packet::Publish(p) = decoded {
            assert_eq!(&*p.topic, "test/topic");
            assert_eq!(p.packet_id, Some(123));
            assert_eq!(p.qos, QoS::AtLeastOnce);
            assert_eq!(&p.payload[..], b"hello world");
        } else {
            panic!("Expected Publish packet");
        }
    }

    #[test]
    fn test_cached_publish_different_packet_ids() {
        let publish = Publish {
            dup: false,
            qos: QoS::AtLeastOnce,
            retain: false,
            topic: Arc::from("sensors/temp"),
            packet_id: Some(1),
            payload: bytes::Bytes::from("25.5"),
            properties: Properties::default(),
        };

        let cached = CachedPublish::new(&publish, ProtocolVersion::V311).unwrap();

        // Write with different packet_ids
        for pid in [100u16, 200, 300, 65535] {
            let mut buf = BytesMut::new();
            cached.write_to(&mut buf, Some(pid), QoS::AtLeastOnce, false, false);

            let mut decoder = Decoder::new();
            decoder.set_protocol_version(ProtocolVersion::V311);
            let (decoded, _) = decoder.decode(&mut buf).unwrap().unwrap();

            if let crate::protocol::Packet::Publish(p) = decoded {
                assert_eq!(p.packet_id, Some(pid));
            } else {
                panic!("Expected Publish packet");
            }
        }
    }

    #[test]
    fn test_cached_publish_flag_modifications() {
        let publish = Publish {
            dup: false,
            qos: QoS::AtLeastOnce,
            retain: false,
            topic: Arc::from("test"),
            packet_id: Some(1),
            payload: bytes::Bytes::from("data"),
            properties: Properties::default(),
        };

        let cached = CachedPublish::new(&publish, ProtocolVersion::V311).unwrap();

        // Test with retain = true
        let mut buf = BytesMut::new();
        cached.write_to(&mut buf, Some(1), QoS::AtLeastOnce, true, false);

        let mut decoder = Decoder::new();
        decoder.set_protocol_version(ProtocolVersion::V311);
        let (decoded, _) = decoder.decode(&mut buf).unwrap().unwrap();

        if let crate::protocol::Packet::Publish(p) = decoded {
            assert!(p.retain);
            assert!(!p.dup);
        } else {
            panic!("Expected Publish packet");
        }

        // Test with dup = true
        let mut buf = BytesMut::new();
        cached.write_to(&mut buf, Some(1), QoS::AtLeastOnce, false, true);

        let (decoded, _) = decoder.decode(&mut buf).unwrap().unwrap();

        if let crate::protocol::Packet::Publish(p) = decoded {
            assert!(!p.retain);
            assert!(p.dup);
        } else {
            panic!("Expected Publish packet");
        }
    }

    #[test]
    fn test_cached_publish_v5_with_properties() {
        let mut props = Properties::default();
        props.message_expiry_interval = Some(3600);
        props.content_type = Some("application/json".to_string());

        let publish = Publish {
            dup: false,
            qos: QoS::ExactlyOnce,
            retain: true,
            topic: Arc::from("data/stream"),
            packet_id: Some(42),
            payload: bytes::Bytes::from(r#"{"temp": 25}"#),
            properties: props,
        };

        let cached = CachedPublish::new(&publish, ProtocolVersion::V5).unwrap();

        let mut buf = BytesMut::new();
        cached.write_to(&mut buf, Some(42), QoS::ExactlyOnce, true, false);

        let mut decoder = Decoder::new();
        decoder.set_protocol_version(ProtocolVersion::V5);
        let (decoded, _) = decoder.decode(&mut buf).unwrap().unwrap();

        if let crate::protocol::Packet::Publish(p) = decoded {
            assert_eq!(&*p.topic, "data/stream");
            assert_eq!(p.properties.message_expiry_interval, Some(3600));
            assert_eq!(p.properties.content_type, Some("application/json".to_string()));
        } else {
            panic!("Expected Publish packet");
        }
    }
}

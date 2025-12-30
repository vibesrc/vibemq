//! MQTT Packet Decoder
//!
//! Decodes MQTT packets for both v3.1.1 and v5.0

use std::sync::Arc;

use bytes::Bytes;

use super::{read_binary, read_string, read_variable_int, MAX_REMAINING_LENGTH};
use crate::protocol::{
    Auth, ConnAck, Connect, DecodeError, Disconnect, Packet, Properties, ProtocolVersion, PubAck,
    PubComp, PubRec, PubRel, Publish, QoS, ReasonCode, SubAck, Subscribe, Subscription,
    SubscriptionOptions, UnsubAck, Unsubscribe, Will,
};

/// MQTT Packet Decoder
pub struct Decoder {
    /// Maximum packet size
    max_packet_size: usize,
    /// Current protocol version (set after CONNECT)
    protocol_version: Option<ProtocolVersion>,
}

impl Decoder {
    pub fn new() -> Self {
        Self {
            max_packet_size: MAX_REMAINING_LENGTH,
            protocol_version: None,
        }
    }

    pub fn with_max_packet_size(mut self, size: usize) -> Self {
        self.max_packet_size = size.min(MAX_REMAINING_LENGTH);
        self
    }

    pub fn set_protocol_version(&mut self, version: ProtocolVersion) {
        self.protocol_version = Some(version);
    }

    pub fn protocol_version(&self) -> Option<ProtocolVersion> {
        self.protocol_version
    }

    /// Decode a packet from the buffer
    /// Returns (packet, bytes_consumed) or error
    pub fn decode(&mut self, buf: &[u8]) -> Result<Option<(Packet, usize)>, DecodeError> {
        if buf.len() < 2 {
            return Ok(None);
        }

        // Parse fixed header
        let first_byte = buf[0];
        let packet_type = first_byte >> 4;
        let flags = first_byte & 0x0F;

        // Read remaining length
        let (remaining_length, len_bytes) = match read_variable_int(&buf[1..]) {
            Ok(r) => r,
            Err(DecodeError::InsufficientData) => return Ok(None),
            Err(e) => return Err(e),
        };

        let total_len = 1 + len_bytes + remaining_length as usize;

        // Check packet size limit
        if remaining_length as usize > self.max_packet_size {
            return Err(DecodeError::PacketTooLarge);
        }

        // Wait for complete packet
        if buf.len() < total_len {
            return Ok(None);
        }

        let payload_start = 1 + len_bytes;
        let payload = &buf[payload_start..total_len];

        let packet = match packet_type {
            1 => self.decode_connect(payload)?,
            2 => self.decode_connack(flags, payload)?,
            3 => self.decode_publish(flags, payload)?,
            4 => self.decode_puback(flags, payload)?,
            5 => self.decode_pubrec(flags, payload)?,
            6 => self.decode_pubrel(flags, payload)?,
            7 => self.decode_pubcomp(flags, payload)?,
            8 => self.decode_subscribe(flags, payload)?,
            9 => self.decode_suback(flags, payload)?,
            10 => self.decode_unsubscribe(flags, payload)?,
            11 => self.decode_unsuback(flags, payload)?,
            12 => {
                if flags != 0 {
                    return Err(DecodeError::InvalidFlags);
                }
                Packet::PingReq
            }
            13 => {
                if flags != 0 {
                    return Err(DecodeError::InvalidFlags);
                }
                Packet::PingResp
            }
            14 => self.decode_disconnect(flags, payload)?,
            15 => self.decode_auth(flags, payload)?,
            _ => return Err(DecodeError::InvalidPacketType(packet_type)),
        };

        Ok(Some((packet, total_len)))
    }

    fn decode_connect(&mut self, payload: &[u8]) -> Result<Packet, DecodeError> {
        let mut pos = 0;

        // Protocol name
        let (protocol_name, len) = read_string(&payload[pos..])?;
        pos += len;

        if protocol_name != "MQTT" && protocol_name != "MQIsdp" {
            return Err(DecodeError::InvalidProtocolName);
        }

        // Protocol version
        if pos >= payload.len() {
            return Err(DecodeError::InsufficientData);
        }
        let version_byte = payload[pos];
        pos += 1;

        let protocol_version = match version_byte {
            3 | 4 => ProtocolVersion::V311,
            5 => ProtocolVersion::V5,
            _ => return Err(DecodeError::InvalidProtocolVersion(version_byte)),
        };

        self.protocol_version = Some(protocol_version);

        // Connect flags
        if pos >= payload.len() {
            return Err(DecodeError::InsufficientData);
        }
        let connect_flags = payload[pos];
        pos += 1;

        // Reserved bit must be 0
        if (connect_flags & 0x01) != 0 {
            return Err(DecodeError::InvalidFlags);
        }

        let clean_start = (connect_flags & 0x02) != 0;
        let will_flag = (connect_flags & 0x04) != 0;
        let will_qos = (connect_flags >> 3) & 0x03;
        let will_retain = (connect_flags & 0x20) != 0;
        let password_flag = (connect_flags & 0x40) != 0;
        let username_flag = (connect_flags & 0x80) != 0;

        // [MQTT-3.1.2-22] If username flag is 0, password flag must be 0
        if !username_flag && password_flag {
            return Err(DecodeError::InvalidFlags);
        }

        // Validate will QoS
        if will_qos > 2 {
            return Err(DecodeError::InvalidQoS(will_qos));
        }

        // If will flag is 0, will QoS and will retain must be 0
        if !will_flag && (will_qos != 0 || will_retain) {
            return Err(DecodeError::InvalidFlags);
        }

        // Keep alive
        if pos + 2 > payload.len() {
            return Err(DecodeError::InsufficientData);
        }
        let keep_alive = u16::from_be_bytes([payload[pos], payload[pos + 1]]);
        pos += 2;

        // Properties (v5.0 only)
        let properties = if protocol_version == ProtocolVersion::V5 {
            let (props, len) = Properties::decode(&payload[pos..])?;
            pos += len;
            props
        } else {
            Properties::default()
        };

        // Client ID
        let (client_id, len) = read_string(&payload[pos..])?;
        pos += len;

        // Will message
        let will = if will_flag {
            // Will properties (v5.0 only)
            let will_properties = if protocol_version == ProtocolVersion::V5 {
                let (props, len) = Properties::decode(&payload[pos..])?;
                pos += len;
                props
            } else {
                Properties::default()
            };

            // Will topic
            let (will_topic, len) = read_string(&payload[pos..])?;
            pos += len;

            // Will payload
            let (will_payload, len) = read_binary(&payload[pos..])?;
            pos += len;

            Some(Will {
                topic: will_topic.to_string(),
                payload: Bytes::copy_from_slice(will_payload),
                qos: QoS::from_u8(will_qos).unwrap(),
                retain: will_retain,
                properties: will_properties,
            })
        } else {
            None
        };

        // Username
        let username = if username_flag {
            let (s, len) = read_string(&payload[pos..])?;
            pos += len;
            Some(s.to_string())
        } else {
            None
        };

        // Password
        let password = if password_flag {
            let (data, _len) = read_binary(&payload[pos..])?;
            Some(Bytes::copy_from_slice(data))
        } else {
            None
        };

        Ok(Packet::Connect(Box::new(Connect {
            protocol_version,
            client_id: client_id.to_string(),
            clean_start,
            keep_alive,
            username,
            password,
            will,
            properties,
        })))
    }

    fn decode_connack(&self, flags: u8, payload: &[u8]) -> Result<Packet, DecodeError> {
        if flags != 0 {
            return Err(DecodeError::InvalidFlags);
        }

        if payload.len() < 2 {
            return Err(DecodeError::InsufficientData);
        }

        let acknowledge_flags = payload[0];
        // Only bit 0 is valid (session present), rest must be 0
        if (acknowledge_flags & 0xFE) != 0 {
            return Err(DecodeError::InvalidFlags);
        }

        let session_present = (acknowledge_flags & 0x01) != 0;
        let reason_byte = payload[1];

        let (reason_code, properties) = match self.protocol_version {
            Some(ProtocolVersion::V5) | None => {
                let reason_code = ReasonCode::from_u8(reason_byte)
                    .ok_or(DecodeError::InvalidReasonCode(reason_byte))?;
                let properties = if payload.len() > 2 {
                    let (props, _) = Properties::decode(&payload[2..])?;
                    props
                } else {
                    Properties::default()
                };
                (reason_code, properties)
            }
            Some(ProtocolVersion::V311) => (
                ReasonCode::from_v3_connack_code(reason_byte),
                Properties::default(),
            ),
        };

        Ok(Packet::ConnAck(ConnAck {
            session_present,
            reason_code,
            properties,
        }))
    }

    fn decode_publish(&self, flags: u8, payload: &[u8]) -> Result<Packet, DecodeError> {
        let dup = (flags & 0x08) != 0;
        let qos_bits = (flags >> 1) & 0x03;
        let retain = (flags & 0x01) != 0;

        let qos = QoS::from_u8(qos_bits).ok_or(DecodeError::InvalidQoS(qos_bits))?;

        // DUP must be 0 for QoS 0
        if qos == QoS::AtMostOnce && dup {
            return Err(DecodeError::MalformedPacket("DUP must be 0 for QoS 0"));
        }

        let mut pos = 0;

        // Topic name
        let (topic, len) = read_string(&payload[pos..])?;
        pos += len;

        // Validate topic (no wildcards allowed in PUBLISH)
        if topic.contains('+') || topic.contains('#') {
            return Err(DecodeError::MalformedPacket("topic contains wildcard"));
        }

        // Packet ID (only for QoS > 0)
        let packet_id = if qos != QoS::AtMostOnce {
            if pos + 2 > payload.len() {
                return Err(DecodeError::InsufficientData);
            }
            let id = u16::from_be_bytes([payload[pos], payload[pos + 1]]);
            if id == 0 {
                return Err(DecodeError::MalformedPacket("packet id cannot be 0"));
            }
            pos += 2;
            Some(id)
        } else {
            None
        };

        // Properties (v5.0 only)
        let properties = match self.protocol_version {
            Some(ProtocolVersion::V5) => {
                let (props, len) = Properties::decode(&payload[pos..])?;
                pos += len;
                props
            }
            _ => Properties::default(),
        };

        // Payload (remainder)
        let message_payload = Bytes::copy_from_slice(&payload[pos..]);

        Ok(Packet::Publish(Publish {
            dup,
            qos,
            retain,
            topic: Arc::from(topic),
            packet_id,
            payload: message_payload,
            properties,
        }))
    }

    fn decode_puback(&self, flags: u8, payload: &[u8]) -> Result<Packet, DecodeError> {
        if flags != 0 {
            return Err(DecodeError::InvalidFlags);
        }

        if payload.len() < 2 {
            return Err(DecodeError::InsufficientData);
        }

        let packet_id = u16::from_be_bytes([payload[0], payload[1]]);

        let (reason_code, properties) = match self.protocol_version {
            Some(ProtocolVersion::V5) => {
                if payload.len() > 2 {
                    let reason_code = ReasonCode::from_u8(payload[2])
                        .ok_or(DecodeError::InvalidReasonCode(payload[2]))?;
                    let properties = if payload.len() > 3 {
                        let (props, _) = Properties::decode(&payload[3..])?;
                        props
                    } else {
                        Properties::default()
                    };
                    (reason_code, properties)
                } else {
                    (ReasonCode::Success, Properties::default())
                }
            }
            _ => (ReasonCode::Success, Properties::default()),
        };

        Ok(Packet::PubAck(PubAck {
            packet_id,
            reason_code,
            properties,
        }))
    }

    fn decode_pubrec(&self, flags: u8, payload: &[u8]) -> Result<Packet, DecodeError> {
        if flags != 0 {
            return Err(DecodeError::InvalidFlags);
        }

        if payload.len() < 2 {
            return Err(DecodeError::InsufficientData);
        }

        let packet_id = u16::from_be_bytes([payload[0], payload[1]]);

        let (reason_code, properties) = match self.protocol_version {
            Some(ProtocolVersion::V5) => {
                if payload.len() > 2 {
                    let reason_code = ReasonCode::from_u8(payload[2])
                        .ok_or(DecodeError::InvalidReasonCode(payload[2]))?;
                    let properties = if payload.len() > 3 {
                        let (props, _) = Properties::decode(&payload[3..])?;
                        props
                    } else {
                        Properties::default()
                    };
                    (reason_code, properties)
                } else {
                    (ReasonCode::Success, Properties::default())
                }
            }
            _ => (ReasonCode::Success, Properties::default()),
        };

        Ok(Packet::PubRec(PubRec {
            packet_id,
            reason_code,
            properties,
        }))
    }

    fn decode_pubrel(&self, flags: u8, payload: &[u8]) -> Result<Packet, DecodeError> {
        // PUBREL must have flags 0010
        if flags != 0x02 {
            return Err(DecodeError::InvalidFlags);
        }

        if payload.len() < 2 {
            return Err(DecodeError::InsufficientData);
        }

        let packet_id = u16::from_be_bytes([payload[0], payload[1]]);

        let (reason_code, properties) = match self.protocol_version {
            Some(ProtocolVersion::V5) => {
                if payload.len() > 2 {
                    let reason_code = ReasonCode::from_u8(payload[2])
                        .ok_or(DecodeError::InvalidReasonCode(payload[2]))?;
                    let properties = if payload.len() > 3 {
                        let (props, _) = Properties::decode(&payload[3..])?;
                        props
                    } else {
                        Properties::default()
                    };
                    (reason_code, properties)
                } else {
                    (ReasonCode::Success, Properties::default())
                }
            }
            _ => (ReasonCode::Success, Properties::default()),
        };

        Ok(Packet::PubRel(PubRel {
            packet_id,
            reason_code,
            properties,
        }))
    }

    fn decode_pubcomp(&self, flags: u8, payload: &[u8]) -> Result<Packet, DecodeError> {
        if flags != 0 {
            return Err(DecodeError::InvalidFlags);
        }

        if payload.len() < 2 {
            return Err(DecodeError::InsufficientData);
        }

        let packet_id = u16::from_be_bytes([payload[0], payload[1]]);

        let (reason_code, properties) = match self.protocol_version {
            Some(ProtocolVersion::V5) => {
                if payload.len() > 2 {
                    let reason_code = ReasonCode::from_u8(payload[2])
                        .ok_or(DecodeError::InvalidReasonCode(payload[2]))?;
                    let properties = if payload.len() > 3 {
                        let (props, _) = Properties::decode(&payload[3..])?;
                        props
                    } else {
                        Properties::default()
                    };
                    (reason_code, properties)
                } else {
                    (ReasonCode::Success, Properties::default())
                }
            }
            _ => (ReasonCode::Success, Properties::default()),
        };

        Ok(Packet::PubComp(PubComp {
            packet_id,
            reason_code,
            properties,
        }))
    }

    fn decode_subscribe(&self, flags: u8, payload: &[u8]) -> Result<Packet, DecodeError> {
        // SUBSCRIBE must have flags 0010
        if flags != 0x02 {
            return Err(DecodeError::InvalidFlags);
        }

        if payload.len() < 2 {
            return Err(DecodeError::InsufficientData);
        }

        let packet_id = u16::from_be_bytes([payload[0], payload[1]]);
        if packet_id == 0 {
            return Err(DecodeError::MalformedPacket("packet id cannot be 0"));
        }

        let mut pos = 2;

        // Properties (v5.0 only)
        let properties = match self.protocol_version {
            Some(ProtocolVersion::V5) => {
                let (props, len) = Properties::decode(&payload[pos..])?;
                pos += len;
                props
            }
            _ => Properties::default(),
        };

        // Subscriptions
        let mut subscriptions = Vec::new();
        while pos < payload.len() {
            let (filter, len) = read_string(&payload[pos..])?;
            pos += len;

            // [MQTT-4.7.0-1] Topic filter cannot be empty
            if filter.is_empty() {
                return Err(DecodeError::MalformedPacket("topic filter cannot be empty"));
            }

            if pos >= payload.len() {
                return Err(DecodeError::InsufficientData);
            }

            let options_byte = payload[pos];
            pos += 1;

            let options = match self.protocol_version {
                Some(ProtocolVersion::V5) => SubscriptionOptions::from_byte(options_byte)
                    .ok_or(DecodeError::InvalidSubscriptionOptions)?,
                _ => {
                    // v3.1.1 only uses QoS bits
                    let qos = QoS::from_u8(options_byte & 0x03)
                        .ok_or(DecodeError::InvalidQoS(options_byte & 0x03))?;
                    SubscriptionOptions {
                        qos,
                        ..Default::default()
                    }
                }
            };

            subscriptions.push(Subscription {
                filter: filter.to_string(),
                options,
            });
        }

        // SUBSCRIBE must have at least one topic filter
        if subscriptions.is_empty() {
            return Err(DecodeError::MalformedPacket(
                "SUBSCRIBE must have at least one topic",
            ));
        }

        Ok(Packet::Subscribe(Subscribe {
            packet_id,
            subscriptions,
            properties,
        }))
    }

    fn decode_suback(&self, flags: u8, payload: &[u8]) -> Result<Packet, DecodeError> {
        if flags != 0 {
            return Err(DecodeError::InvalidFlags);
        }

        if payload.len() < 3 {
            return Err(DecodeError::InsufficientData);
        }

        let packet_id = u16::from_be_bytes([payload[0], payload[1]]);
        let mut pos = 2;

        // Properties (v5.0 only)
        let properties = match self.protocol_version {
            Some(ProtocolVersion::V5) => {
                let (props, len) = Properties::decode(&payload[pos..])?;
                pos += len;
                props
            }
            _ => Properties::default(),
        };

        // Reason codes
        let mut reason_codes = Vec::new();
        while pos < payload.len() {
            let code = payload[pos];
            pos += 1;

            let reason_code = match self.protocol_version {
                Some(ProtocolVersion::V5) => {
                    ReasonCode::from_u8(code).ok_or(DecodeError::InvalidReasonCode(code))?
                }
                _ => {
                    // v3.1.1 return codes: 0x00-0x02 for granted QoS, 0x80 for failure
                    match code {
                        0x00 => ReasonCode::Success,
                        0x01 => ReasonCode::GrantedQoS1,
                        0x02 => ReasonCode::GrantedQoS2,
                        0x80 => ReasonCode::UnspecifiedError,
                        _ => return Err(DecodeError::InvalidReasonCode(code)),
                    }
                }
            };
            reason_codes.push(reason_code);
        }

        Ok(Packet::SubAck(SubAck {
            packet_id,
            reason_codes,
            properties,
        }))
    }

    fn decode_unsubscribe(&self, flags: u8, payload: &[u8]) -> Result<Packet, DecodeError> {
        // UNSUBSCRIBE must have flags 0010
        if flags != 0x02 {
            return Err(DecodeError::InvalidFlags);
        }

        if payload.len() < 2 {
            return Err(DecodeError::InsufficientData);
        }

        let packet_id = u16::from_be_bytes([payload[0], payload[1]]);
        if packet_id == 0 {
            return Err(DecodeError::MalformedPacket("packet id cannot be 0"));
        }

        let mut pos = 2;

        // Properties (v5.0 only)
        let properties = match self.protocol_version {
            Some(ProtocolVersion::V5) => {
                let (props, len) = Properties::decode(&payload[pos..])?;
                pos += len;
                props
            }
            _ => Properties::default(),
        };

        // Topic filters
        let mut filters = Vec::new();
        while pos < payload.len() {
            let (filter, len) = read_string(&payload[pos..])?;
            pos += len;

            // [MQTT-4.7.0-1] Topic filter cannot be empty
            if filter.is_empty() {
                return Err(DecodeError::MalformedPacket("topic filter cannot be empty"));
            }

            filters.push(filter.to_string());
        }

        // UNSUBSCRIBE must have at least one topic filter
        if filters.is_empty() {
            return Err(DecodeError::MalformedPacket(
                "UNSUBSCRIBE must have at least one topic",
            ));
        }

        Ok(Packet::Unsubscribe(Unsubscribe {
            packet_id,
            filters,
            properties,
        }))
    }

    fn decode_unsuback(&self, flags: u8, payload: &[u8]) -> Result<Packet, DecodeError> {
        if flags != 0 {
            return Err(DecodeError::InvalidFlags);
        }

        if payload.len() < 2 {
            return Err(DecodeError::InsufficientData);
        }

        let packet_id = u16::from_be_bytes([payload[0], payload[1]]);

        let (properties, reason_codes) = match self.protocol_version {
            Some(ProtocolVersion::V5) => {
                let mut pos = 2;
                let (props, len) = Properties::decode(&payload[pos..])?;
                pos += len;

                let mut codes = Vec::new();
                while pos < payload.len() {
                    let code = ReasonCode::from_u8(payload[pos])
                        .ok_or(DecodeError::InvalidReasonCode(payload[pos]))?;
                    codes.push(code);
                    pos += 1;
                }
                (props, codes)
            }
            _ => (Properties::default(), Vec::new()),
        };

        Ok(Packet::UnsubAck(UnsubAck {
            packet_id,
            reason_codes,
            properties,
        }))
    }

    fn decode_disconnect(&self, flags: u8, payload: &[u8]) -> Result<Packet, DecodeError> {
        if flags != 0 {
            return Err(DecodeError::InvalidFlags);
        }

        match self.protocol_version {
            Some(ProtocolVersion::V5) => {
                if payload.is_empty() {
                    return Ok(Packet::Disconnect(Disconnect::default()));
                }

                let reason_code = ReasonCode::from_u8(payload[0])
                    .ok_or(DecodeError::InvalidReasonCode(payload[0]))?;

                let properties = if payload.len() > 1 {
                    let (props, _) = Properties::decode(&payload[1..])?;
                    props
                } else {
                    Properties::default()
                };

                Ok(Packet::Disconnect(Disconnect {
                    reason_code,
                    properties,
                }))
            }
            _ => {
                // v3.1.1 DISCONNECT has no payload
                if !payload.is_empty() {
                    return Err(DecodeError::MalformedPacket(
                        "v3.1.1 DISCONNECT has no payload",
                    ));
                }
                Ok(Packet::Disconnect(Disconnect::default()))
            }
        }
    }

    fn decode_auth(&self, flags: u8, payload: &[u8]) -> Result<Packet, DecodeError> {
        if flags != 0 {
            return Err(DecodeError::InvalidFlags);
        }

        // AUTH is v5.0 only
        if self.protocol_version != Some(ProtocolVersion::V5) {
            return Err(DecodeError::InvalidPacketType(15));
        }

        if payload.is_empty() {
            return Ok(Packet::Auth(Auth::default()));
        }

        let reason_code =
            ReasonCode::from_u8(payload[0]).ok_or(DecodeError::InvalidReasonCode(payload[0]))?;

        let properties = if payload.len() > 1 {
            let (props, _) = Properties::decode(&payload[1..])?;
            props
        } else {
            Properties::default()
        };

        Ok(Packet::Auth(Auth {
            reason_code,
            properties,
        }))
    }
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

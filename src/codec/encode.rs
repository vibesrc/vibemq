//! MQTT Packet Encoder
//!
//! Encodes MQTT packets for both v3.1.1 and v5.0

use bytes::{BufMut, BytesMut};

use super::{variable_int_len, write_binary, write_string, write_variable_int};
use crate::protocol::{
    Auth, ConnAck, Connect, Disconnect, EncodeError, Packet, ProtocolVersion, PubAck, PubComp,
    PubRec, PubRel, Publish, QoS, ReasonCode, SubAck, Subscribe, UnsubAck, Unsubscribe,
};

/// MQTT Packet Encoder
pub struct Encoder {
    protocol_version: ProtocolVersion,
}

impl Encoder {
    pub fn new(version: ProtocolVersion) -> Self {
        Self {
            protocol_version: version,
        }
    }

    pub fn set_protocol_version(&mut self, version: ProtocolVersion) {
        self.protocol_version = version;
    }

    /// Encode a packet to the buffer
    pub fn encode(&self, packet: &Packet, buf: &mut BytesMut) -> Result<(), EncodeError> {
        match packet {
            Packet::Connect(p) => self.encode_connect(p, buf),
            Packet::ConnAck(p) => self.encode_connack(p, buf),
            Packet::Publish(p) => self.encode_publish(p, buf),
            Packet::PubAck(p) => self.encode_puback(p, buf),
            Packet::PubRec(p) => self.encode_pubrec(p, buf),
            Packet::PubRel(p) => self.encode_pubrel(p, buf),
            Packet::PubComp(p) => self.encode_pubcomp(p, buf),
            Packet::Subscribe(p) => self.encode_subscribe(p, buf),
            Packet::SubAck(p) => self.encode_suback(p, buf),
            Packet::Unsubscribe(p) => self.encode_unsubscribe(p, buf),
            Packet::UnsubAck(p) => self.encode_unsuback(p, buf),
            Packet::PingReq => {
                buf.put_u8(0xC0); // PINGREQ type + flags
                buf.put_u8(0x00); // Remaining length
                Ok(())
            }
            Packet::PingResp => {
                buf.put_u8(0xD0); // PINGRESP type + flags
                buf.put_u8(0x00); // Remaining length
                Ok(())
            }
            Packet::Disconnect(p) => self.encode_disconnect(p, buf),
            Packet::Auth(p) => self.encode_auth(p, buf),
        }
    }

    fn encode_connect(&self, packet: &Connect, buf: &mut BytesMut) -> Result<(), EncodeError> {
        // Calculate remaining length
        let mut remaining_length = 0;

        // Protocol name (4 bytes for "MQTT") + length prefix (2 bytes)
        remaining_length += 6;
        // Protocol version (1 byte)
        remaining_length += 1;
        // Connect flags (1 byte)
        remaining_length += 1;
        // Keep alive (2 bytes)
        remaining_length += 2;

        // Properties (v5.0 only)
        let _props_len = if packet.protocol_version == ProtocolVersion::V5 {
            let len = packet.properties.encoded_size();
            remaining_length += variable_int_len(len as u32) + len;
            len
        } else {
            0
        };

        // Client ID
        remaining_length += 2 + packet.client_id.len();

        // Will message
        if let Some(ref will) = packet.will {
            // Will properties (v5.0 only)
            if packet.protocol_version == ProtocolVersion::V5 {
                let will_props_len = will.properties.encoded_size();
                remaining_length += variable_int_len(will_props_len as u32) + will_props_len;
            }
            // Will topic
            remaining_length += 2 + will.topic.len();
            // Will payload
            remaining_length += 2 + will.payload.len();
        }

        // Username
        if let Some(ref username) = packet.username {
            remaining_length += 2 + username.len();
        }

        // Password
        if let Some(ref password) = packet.password {
            remaining_length += 2 + password.len();
        }

        // Fixed header
        buf.put_u8(0x10); // CONNECT type + flags (0001 0000)
        write_variable_int(buf, remaining_length as u32)?;

        // Protocol name
        write_string(buf, "MQTT")?;

        // Protocol version
        buf.put_u8(packet.protocol_version as u8);

        // Connect flags
        let mut connect_flags: u8 = 0;
        if packet.clean_start {
            connect_flags |= 0x02;
        }
        if let Some(ref will) = packet.will {
            connect_flags |= 0x04;
            connect_flags |= (will.qos as u8) << 3;
            if will.retain {
                connect_flags |= 0x20;
            }
        }
        if packet.password.is_some() {
            connect_flags |= 0x40;
        }
        if packet.username.is_some() {
            connect_flags |= 0x80;
        }
        buf.put_u8(connect_flags);

        // Keep alive
        buf.put_u16(packet.keep_alive);

        // Properties (v5.0 only)
        if packet.protocol_version == ProtocolVersion::V5 {
            packet.properties.encode(buf)?;
        }

        // Client ID
        write_string(buf, &packet.client_id)?;

        // Will message
        if let Some(ref will) = packet.will {
            // Will properties (v5.0 only)
            if packet.protocol_version == ProtocolVersion::V5 {
                will.properties.encode(buf)?;
            }
            // Will topic
            write_string(buf, &will.topic)?;
            // Will payload
            write_binary(buf, &will.payload)?;
        }

        // Username
        if let Some(ref username) = packet.username {
            write_string(buf, username)?;
        }

        // Password
        if let Some(ref password) = packet.password {
            write_binary(buf, password)?;
        }

        Ok(())
    }

    fn encode_connack(&self, packet: &ConnAck, buf: &mut BytesMut) -> Result<(), EncodeError> {
        let is_v5 = self.protocol_version == ProtocolVersion::V5;

        // Calculate remaining length
        let mut remaining_length = 2; // acknowledge flags + reason code

        if is_v5 {
            let props_len = packet.properties.encoded_size();
            remaining_length += variable_int_len(props_len as u32) + props_len;
        }

        // Fixed header
        buf.put_u8(0x20); // CONNACK type + flags (0010 0000)
        write_variable_int(buf, remaining_length as u32)?;

        // Session present flag
        buf.put_u8(if packet.session_present { 0x01 } else { 0x00 });

        // Reason code
        if is_v5 {
            buf.put_u8(packet.reason_code as u8);
            packet.properties.encode(buf)?;
        } else {
            buf.put_u8(packet.reason_code.to_v3_connack_code());
        }

        Ok(())
    }

    fn encode_publish(&self, packet: &Publish, buf: &mut BytesMut) -> Result<(), EncodeError> {
        let is_v5 = self.protocol_version == ProtocolVersion::V5;

        // Calculate remaining length
        let mut remaining_length = 2 + packet.topic.len(); // topic length prefix + topic

        if packet.qos != QoS::AtMostOnce {
            remaining_length += 2; // packet identifier
        }

        if is_v5 {
            let props_len = packet.properties.encoded_size();
            remaining_length += variable_int_len(props_len as u32) + props_len;
        }

        remaining_length += packet.payload.len();

        // Fixed header
        let mut first_byte: u8 = 0x30; // PUBLISH type (0011)
        if packet.dup {
            first_byte |= 0x08;
        }
        first_byte |= (packet.qos as u8) << 1;
        if packet.retain {
            first_byte |= 0x01;
        }
        buf.put_u8(first_byte);
        write_variable_int(buf, remaining_length as u32)?;

        // Topic name
        write_string(buf, &packet.topic)?;

        // Packet identifier (only for QoS > 0)
        if let Some(packet_id) = packet.packet_id {
            buf.put_u16(packet_id);
        }

        // Properties (v5.0 only)
        if is_v5 {
            packet.properties.encode(buf)?;
        }

        // Payload
        buf.put_slice(&packet.payload);

        Ok(())
    }

    fn encode_puback(&self, packet: &PubAck, buf: &mut BytesMut) -> Result<(), EncodeError> {
        let is_v5 = self.protocol_version == ProtocolVersion::V5;

        if is_v5 {
            // For v5.0, we can omit reason code and properties if success and no properties
            let has_reason =
                packet.reason_code != ReasonCode::Success || !packet.properties.is_empty();

            if has_reason {
                let props_len = packet.properties.encoded_size();
                let has_props = props_len > 0;
                let remaining_length = if has_props {
                    2 + 1 + variable_int_len(props_len as u32) + props_len
                } else {
                    2 + 1 // packet_id + reason_code
                };

                buf.put_u8(0x40);
                write_variable_int(buf, remaining_length as u32)?;
                buf.put_u16(packet.packet_id);
                buf.put_u8(packet.reason_code as u8);
                if has_props {
                    packet.properties.encode(buf)?;
                }
            } else {
                buf.put_u8(0x40);
                buf.put_u8(0x02);
                buf.put_u16(packet.packet_id);
            }
        } else {
            // v3.1.1
            buf.put_u8(0x40);
            buf.put_u8(0x02);
            buf.put_u16(packet.packet_id);
        }

        Ok(())
    }

    fn encode_pubrec(&self, packet: &PubRec, buf: &mut BytesMut) -> Result<(), EncodeError> {
        let is_v5 = self.protocol_version == ProtocolVersion::V5;

        if is_v5 {
            let has_reason =
                packet.reason_code != ReasonCode::Success || !packet.properties.is_empty();

            if has_reason {
                let props_len = packet.properties.encoded_size();
                let has_props = props_len > 0;
                let remaining_length = if has_props {
                    2 + 1 + variable_int_len(props_len as u32) + props_len
                } else {
                    2 + 1
                };

                buf.put_u8(0x50);
                write_variable_int(buf, remaining_length as u32)?;
                buf.put_u16(packet.packet_id);
                buf.put_u8(packet.reason_code as u8);
                if has_props {
                    packet.properties.encode(buf)?;
                }
            } else {
                buf.put_u8(0x50);
                buf.put_u8(0x02);
                buf.put_u16(packet.packet_id);
            }
        } else {
            buf.put_u8(0x50);
            buf.put_u8(0x02);
            buf.put_u16(packet.packet_id);
        }

        Ok(())
    }

    fn encode_pubrel(&self, packet: &PubRel, buf: &mut BytesMut) -> Result<(), EncodeError> {
        let is_v5 = self.protocol_version == ProtocolVersion::V5;

        if is_v5 {
            let has_reason =
                packet.reason_code != ReasonCode::Success || !packet.properties.is_empty();

            if has_reason {
                let props_len = packet.properties.encoded_size();
                let has_props = props_len > 0;
                let remaining_length = if has_props {
                    2 + 1 + variable_int_len(props_len as u32) + props_len
                } else {
                    2 + 1
                };

                buf.put_u8(0x62); // PUBREL with flags 0010
                write_variable_int(buf, remaining_length as u32)?;
                buf.put_u16(packet.packet_id);
                buf.put_u8(packet.reason_code as u8);
                if has_props {
                    packet.properties.encode(buf)?;
                }
            } else {
                buf.put_u8(0x62);
                buf.put_u8(0x02);
                buf.put_u16(packet.packet_id);
            }
        } else {
            buf.put_u8(0x62); // PUBREL with flags 0010
            buf.put_u8(0x02);
            buf.put_u16(packet.packet_id);
        }

        Ok(())
    }

    fn encode_pubcomp(&self, packet: &PubComp, buf: &mut BytesMut) -> Result<(), EncodeError> {
        let is_v5 = self.protocol_version == ProtocolVersion::V5;

        if is_v5 {
            let has_reason =
                packet.reason_code != ReasonCode::Success || !packet.properties.is_empty();

            if has_reason {
                let props_len = packet.properties.encoded_size();
                let has_props = props_len > 0;
                let remaining_length = if has_props {
                    2 + 1 + variable_int_len(props_len as u32) + props_len
                } else {
                    2 + 1
                };

                buf.put_u8(0x70);
                write_variable_int(buf, remaining_length as u32)?;
                buf.put_u16(packet.packet_id);
                buf.put_u8(packet.reason_code as u8);
                if has_props {
                    packet.properties.encode(buf)?;
                }
            } else {
                buf.put_u8(0x70);
                buf.put_u8(0x02);
                buf.put_u16(packet.packet_id);
            }
        } else {
            buf.put_u8(0x70);
            buf.put_u8(0x02);
            buf.put_u16(packet.packet_id);
        }

        Ok(())
    }

    fn encode_subscribe(&self, packet: &Subscribe, buf: &mut BytesMut) -> Result<(), EncodeError> {
        let is_v5 = self.protocol_version == ProtocolVersion::V5;

        // Calculate remaining length
        let mut remaining_length = 2; // packet identifier

        if is_v5 {
            let props_len = packet.properties.encoded_size();
            remaining_length += variable_int_len(props_len as u32) + props_len;
        }

        for sub in &packet.subscriptions {
            remaining_length += 2 + sub.filter.len() + 1; // string + options byte
        }

        // Fixed header
        buf.put_u8(0x82); // SUBSCRIBE type with flags 0010
        write_variable_int(buf, remaining_length as u32)?;

        // Packet identifier
        buf.put_u16(packet.packet_id);

        // Properties (v5.0 only)
        if is_v5 {
            packet.properties.encode(buf)?;
        }

        // Subscriptions
        for sub in &packet.subscriptions {
            write_string(buf, &sub.filter)?;
            if is_v5 {
                buf.put_u8(sub.options.to_byte());
            } else {
                buf.put_u8(sub.options.qos as u8);
            }
        }

        Ok(())
    }

    fn encode_suback(&self, packet: &SubAck, buf: &mut BytesMut) -> Result<(), EncodeError> {
        let is_v5 = self.protocol_version == ProtocolVersion::V5;

        // Calculate remaining length
        let mut remaining_length = 2; // packet identifier

        if is_v5 {
            let props_len = packet.properties.encoded_size();
            remaining_length += variable_int_len(props_len as u32) + props_len;
        }

        remaining_length += packet.reason_codes.len();

        // Fixed header
        buf.put_u8(0x90); // SUBACK type
        write_variable_int(buf, remaining_length as u32)?;

        // Packet identifier
        buf.put_u16(packet.packet_id);

        // Properties (v5.0 only)
        if is_v5 {
            packet.properties.encode(buf)?;
        }

        // Reason codes
        for code in &packet.reason_codes {
            if is_v5 {
                buf.put_u8(*code as u8);
            } else {
                // v3.1.1 return codes
                let v3_code = match code {
                    ReasonCode::Success => 0x00,
                    ReasonCode::GrantedQoS1 => 0x01,
                    ReasonCode::GrantedQoS2 => 0x02,
                    _ => 0x80, // Failure
                };
                buf.put_u8(v3_code);
            }
        }

        Ok(())
    }

    fn encode_unsubscribe(
        &self,
        packet: &Unsubscribe,
        buf: &mut BytesMut,
    ) -> Result<(), EncodeError> {
        let is_v5 = self.protocol_version == ProtocolVersion::V5;

        // Calculate remaining length
        let mut remaining_length = 2; // packet identifier

        if is_v5 {
            let props_len = packet.properties.encoded_size();
            remaining_length += variable_int_len(props_len as u32) + props_len;
        }

        for filter in &packet.filters {
            remaining_length += 2 + filter.len();
        }

        // Fixed header
        buf.put_u8(0xA2); // UNSUBSCRIBE type with flags 0010
        write_variable_int(buf, remaining_length as u32)?;

        // Packet identifier
        buf.put_u16(packet.packet_id);

        // Properties (v5.0 only)
        if is_v5 {
            packet.properties.encode(buf)?;
        }

        // Topic filters
        for filter in &packet.filters {
            write_string(buf, filter)?;
        }

        Ok(())
    }

    fn encode_unsuback(&self, packet: &UnsubAck, buf: &mut BytesMut) -> Result<(), EncodeError> {
        let is_v5 = self.protocol_version == ProtocolVersion::V5;

        if is_v5 {
            // Calculate remaining length
            let props_len = packet.properties.encoded_size();
            let remaining_length =
                2 + variable_int_len(props_len as u32) + props_len + packet.reason_codes.len();

            buf.put_u8(0xB0); // UNSUBACK type
            write_variable_int(buf, remaining_length as u32)?;
            buf.put_u16(packet.packet_id);
            packet.properties.encode(buf)?;

            for code in &packet.reason_codes {
                buf.put_u8(*code as u8);
            }
        } else {
            // v3.1.1 UNSUBACK has no payload
            buf.put_u8(0xB0);
            buf.put_u8(0x02);
            buf.put_u16(packet.packet_id);
        }

        Ok(())
    }

    fn encode_disconnect(
        &self,
        packet: &Disconnect,
        buf: &mut BytesMut,
    ) -> Result<(), EncodeError> {
        let is_v5 = self.protocol_version == ProtocolVersion::V5;

        if is_v5 {
            let has_reason =
                packet.reason_code != ReasonCode::Success || !packet.properties.is_empty();

            if has_reason {
                let props_len = packet.properties.encoded_size();
                let has_props = props_len > 0;
                let remaining_length = if has_props {
                    1 + variable_int_len(props_len as u32) + props_len
                } else {
                    1 // just reason code
                };

                buf.put_u8(0xE0); // DISCONNECT type
                write_variable_int(buf, remaining_length as u32)?;
                buf.put_u8(packet.reason_code as u8);
                if has_props {
                    packet.properties.encode(buf)?;
                }
            } else {
                buf.put_u8(0xE0);
                buf.put_u8(0x00);
            }
        } else {
            // v3.1.1 DISCONNECT has no payload
            buf.put_u8(0xE0);
            buf.put_u8(0x00);
        }

        Ok(())
    }

    fn encode_auth(&self, packet: &Auth, buf: &mut BytesMut) -> Result<(), EncodeError> {
        // AUTH is v5.0 only
        if self.protocol_version != ProtocolVersion::V5 {
            return Err(EncodeError::PacketTooLarge); // TODO: better error type
        }

        let has_reason = packet.reason_code != ReasonCode::Success || !packet.properties.is_empty();

        if has_reason {
            let props_len = packet.properties.encoded_size();
            let has_props = props_len > 0;
            let remaining_length = if has_props {
                1 + variable_int_len(props_len as u32) + props_len
            } else {
                1
            };

            buf.put_u8(0xF0); // AUTH type
            write_variable_int(buf, remaining_length as u32)?;
            buf.put_u8(packet.reason_code as u8);
            if has_props {
                packet.properties.encode(buf)?;
            }
        } else {
            buf.put_u8(0xF0);
            buf.put_u8(0x00);
        }

        Ok(())
    }
}

impl Default for Encoder {
    fn default() -> Self {
        Self::new(ProtocolVersion::V5)
    }
}

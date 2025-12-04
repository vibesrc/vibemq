//! MQTT Protocol definitions and types
//!
//! Defines core protocol types used across MQTT v3.1.1 and v5.0

mod error;
mod packet;
mod properties;
mod reason;

pub use error::{DecodeError, EncodeError, ProtocolError};
pub use packet::*;
pub use properties::{Properties, Property};
pub use reason::ReasonCode;

/// MQTT Protocol Version
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ProtocolVersion {
    /// MQTT v3.1.1 (protocol level 4)
    V311 = 4,
    /// MQTT v5.0 (protocol level 5)
    V5 = 5,
}

impl ProtocolVersion {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            4 => Some(ProtocolVersion::V311),
            5 => Some(ProtocolVersion::V5),
            _ => None,
        }
    }
}

/// Quality of Service levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(u8)]
pub enum QoS {
    /// At most once delivery
    #[default]
    AtMostOnce = 0,
    /// At least once delivery
    AtLeastOnce = 1,
    /// Exactly once delivery
    ExactlyOnce = 2,
}

impl QoS {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(QoS::AtMostOnce),
            1 => Some(QoS::AtLeastOnce),
            2 => Some(QoS::ExactlyOnce),
            _ => None,
        }
    }

    /// Returns the minimum of two QoS levels (for subscription matching)
    pub fn min(self, other: Self) -> Self {
        if (self as u8) < (other as u8) {
            self
        } else {
            other
        }
    }
}

/// Retain handling options (MQTT v5.0)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum RetainHandling {
    /// Send retained messages at subscription time
    #[default]
    SendAtSubscribe = 0,
    /// Send retained messages only for new subscriptions
    SendAtSubscribeIfNew = 1,
    /// Do not send retained messages
    DoNotSend = 2,
}

impl RetainHandling {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(RetainHandling::SendAtSubscribe),
            1 => Some(RetainHandling::SendAtSubscribeIfNew),
            2 => Some(RetainHandling::DoNotSend),
            _ => None,
        }
    }
}

/// Subscription options for MQTT v5.0
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubscriptionOptions {
    pub qos: QoS,
    pub no_local: bool,
    pub retain_as_published: bool,
    pub retain_handling: RetainHandling,
}

impl Default for SubscriptionOptions {
    fn default() -> Self {
        Self {
            qos: QoS::AtMostOnce,
            no_local: false,
            retain_as_published: false,
            retain_handling: RetainHandling::SendAtSubscribe,
        }
    }
}

impl SubscriptionOptions {
    pub fn from_byte(byte: u8) -> Option<Self> {
        let qos = QoS::from_u8(byte & 0x03)?;
        let no_local = (byte & 0x04) != 0;
        let retain_as_published = (byte & 0x08) != 0;
        let retain_handling = RetainHandling::from_u8((byte >> 4) & 0x03)?;

        // Check reserved bits are zero
        if (byte & 0xC0) != 0 {
            return None;
        }

        Some(Self {
            qos,
            no_local,
            retain_as_published,
            retain_handling,
        })
    }

    pub fn to_byte(self) -> u8 {
        (self.qos as u8)
            | ((self.no_local as u8) << 2)
            | ((self.retain_as_published as u8) << 3)
            | ((self.retain_handling as u8) << 4)
    }
}

/// MQTT Packet Type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PacketType {
    Connect = 1,
    ConnAck = 2,
    Publish = 3,
    PubAck = 4,
    PubRec = 5,
    PubRel = 6,
    PubComp = 7,
    Subscribe = 8,
    SubAck = 9,
    Unsubscribe = 10,
    UnsubAck = 11,
    PingReq = 12,
    PingResp = 13,
    Disconnect = 14,
    Auth = 15,
}

impl PacketType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(PacketType::Connect),
            2 => Some(PacketType::ConnAck),
            3 => Some(PacketType::Publish),
            4 => Some(PacketType::PubAck),
            5 => Some(PacketType::PubRec),
            6 => Some(PacketType::PubRel),
            7 => Some(PacketType::PubComp),
            8 => Some(PacketType::Subscribe),
            9 => Some(PacketType::SubAck),
            10 => Some(PacketType::Unsubscribe),
            11 => Some(PacketType::UnsubAck),
            12 => Some(PacketType::PingReq),
            13 => Some(PacketType::PingResp),
            14 => Some(PacketType::Disconnect),
            15 => Some(PacketType::Auth),
            _ => None,
        }
    }
}

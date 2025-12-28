//! MQTT Packet Definitions
//!
//! Unified packet types supporting both MQTT v3.1.1 and v5.0

use std::sync::Arc;

use bytes::Bytes;

use super::{Properties, ProtocolVersion, QoS, ReasonCode, SubscriptionOptions};

/// MQTT Packet - unified representation for v3.1.1 and v5.0
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum Packet {
    Connect(Box<Connect>),
    ConnAck(ConnAck),
    Publish(Publish),
    PubAck(PubAck),
    PubRec(PubRec),
    PubRel(PubRel),
    PubComp(PubComp),
    Subscribe(Subscribe),
    SubAck(SubAck),
    Unsubscribe(Unsubscribe),
    UnsubAck(UnsubAck),
    PingReq,
    PingResp,
    Disconnect(Disconnect),
    Auth(Auth),
}

impl Packet {
    /// Get packet type as u8
    pub fn packet_type(&self) -> u8 {
        match self {
            Packet::Connect(_) => 1,
            Packet::ConnAck(_) => 2,
            Packet::Publish(_) => 3,
            Packet::PubAck(_) => 4,
            Packet::PubRec(_) => 5,
            Packet::PubRel(_) => 6,
            Packet::PubComp(_) => 7,
            Packet::Subscribe(_) => 8,
            Packet::SubAck(_) => 9,
            Packet::Unsubscribe(_) => 10,
            Packet::UnsubAck(_) => 11,
            Packet::PingReq => 12,
            Packet::PingResp => 13,
            Packet::Disconnect(_) => 14,
            Packet::Auth(_) => 15,
        }
    }
}

/// CONNECT packet (client -> server)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connect {
    /// Protocol version (determines v3.1.1 or v5.0 behavior)
    pub protocol_version: ProtocolVersion,
    /// Client identifier
    pub client_id: String,
    /// Clean session (v3.1.1) / Clean start (v5.0)
    pub clean_start: bool,
    /// Keep alive interval in seconds
    pub keep_alive: u16,
    /// Username (optional)
    pub username: Option<String>,
    /// Password (optional)
    pub password: Option<Bytes>,
    /// Will message (optional)
    pub will: Option<Will>,
    /// Properties (v5.0 only)
    pub properties: Properties,
}

impl Default for Connect {
    fn default() -> Self {
        Self {
            protocol_version: ProtocolVersion::V5,
            client_id: String::new(),
            clean_start: true,
            keep_alive: 60,
            username: None,
            password: None,
            will: None,
            properties: Properties::default(),
        }
    }
}

/// Will message configuration
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Will {
    /// Will topic
    pub topic: String,
    /// Will payload
    pub payload: Bytes,
    /// Will QoS
    pub qos: QoS,
    /// Will retain flag
    pub retain: bool,
    /// Will properties (v5.0 only)
    pub properties: Properties,
}

/// CONNACK packet (server -> client)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnAck {
    /// Session present flag
    pub session_present: bool,
    /// Reason code (v5.0) / Return code (v3.1.1)
    pub reason_code: ReasonCode,
    /// Properties (v5.0 only)
    pub properties: Properties,
}

impl Default for ConnAck {
    fn default() -> Self {
        Self {
            session_present: false,
            reason_code: ReasonCode::Success,
            properties: Properties::default(),
        }
    }
}

/// PUBLISH packet (bidirectional)
///
/// The topic field uses `Arc<str>` for efficient fan-out: when routing a message
/// to multiple subscribers, cloning the topic is O(1) instead of O(n) for String.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publish {
    /// Duplicate delivery flag
    pub dup: bool,
    /// Quality of service
    pub qos: QoS,
    /// Retain flag
    pub retain: bool,
    /// Topic name (Arc<str> for cheap cloning during fan-out)
    pub topic: Arc<str>,
    /// Packet identifier (present only for QoS > 0)
    pub packet_id: Option<u16>,
    /// Payload
    pub payload: Bytes,
    /// Properties (v5.0 only)
    pub properties: Properties,
}

impl Default for Publish {
    fn default() -> Self {
        Self {
            dup: false,
            qos: QoS::AtMostOnce,
            retain: false,
            topic: Arc::from(""),
            packet_id: None,
            payload: Bytes::new(),
            properties: Properties::default(),
        }
    }
}

/// PUBACK packet (bidirectional, QoS 1)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PubAck {
    /// Packet identifier
    pub packet_id: u16,
    /// Reason code (v5.0 only)
    pub reason_code: ReasonCode,
    /// Properties (v5.0 only)
    pub properties: Properties,
}

impl PubAck {
    pub fn new(packet_id: u16) -> Self {
        Self {
            packet_id,
            reason_code: ReasonCode::Success,
            properties: Properties::default(),
        }
    }
}

/// PUBREC packet (bidirectional, QoS 2 step 1)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PubRec {
    /// Packet identifier
    pub packet_id: u16,
    /// Reason code (v5.0 only)
    pub reason_code: ReasonCode,
    /// Properties (v5.0 only)
    pub properties: Properties,
}

impl PubRec {
    pub fn new(packet_id: u16) -> Self {
        Self {
            packet_id,
            reason_code: ReasonCode::Success,
            properties: Properties::default(),
        }
    }
}

/// PUBREL packet (bidirectional, QoS 2 step 2)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PubRel {
    /// Packet identifier
    pub packet_id: u16,
    /// Reason code (v5.0 only)
    pub reason_code: ReasonCode,
    /// Properties (v5.0 only)
    pub properties: Properties,
}

impl PubRel {
    pub fn new(packet_id: u16) -> Self {
        Self {
            packet_id,
            reason_code: ReasonCode::Success,
            properties: Properties::default(),
        }
    }
}

/// PUBCOMP packet (bidirectional, QoS 2 step 3)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PubComp {
    /// Packet identifier
    pub packet_id: u16,
    /// Reason code (v5.0 only)
    pub reason_code: ReasonCode,
    /// Properties (v5.0 only)
    pub properties: Properties,
}

impl PubComp {
    pub fn new(packet_id: u16) -> Self {
        Self {
            packet_id,
            reason_code: ReasonCode::Success,
            properties: Properties::default(),
        }
    }
}

/// Subscription request with options
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscription {
    /// Topic filter
    pub filter: String,
    /// Subscription options
    pub options: SubscriptionOptions,
}

/// SUBSCRIBE packet (client -> server)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscribe {
    /// Packet identifier
    pub packet_id: u16,
    /// Subscriptions
    pub subscriptions: Vec<Subscription>,
    /// Properties (v5.0 only)
    pub properties: Properties,
}

/// SUBACK packet (server -> client)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubAck {
    /// Packet identifier
    pub packet_id: u16,
    /// Reason codes for each subscription
    pub reason_codes: Vec<ReasonCode>,
    /// Properties (v5.0 only)
    pub properties: Properties,
}

/// UNSUBSCRIBE packet (client -> server)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsubscribe {
    /// Packet identifier
    pub packet_id: u16,
    /// Topic filters to unsubscribe from
    pub filters: Vec<String>,
    /// Properties (v5.0 only)
    pub properties: Properties,
}

/// UNSUBACK packet (server -> client)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsubAck {
    /// Packet identifier
    pub packet_id: u16,
    /// Reason codes for each unsubscription (v5.0 only, v3.1.1 has no payload)
    pub reason_codes: Vec<ReasonCode>,
    /// Properties (v5.0 only)
    pub properties: Properties,
}

/// DISCONNECT packet (bidirectional in v5.0, client -> server in v3.1.1)
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Disconnect {
    /// Reason code (v5.0 only)
    pub reason_code: ReasonCode,
    /// Properties (v5.0 only)
    pub properties: Properties,
}

/// AUTH packet (v5.0 only)
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Auth {
    /// Reason code
    pub reason_code: ReasonCode,
    /// Properties
    pub properties: Properties,
}

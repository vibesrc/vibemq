//! Serializable data models for persistence.
//!
//! These are storage-friendly versions of runtime types that can be
//! serialized with bincode.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bincode::{Decode, Encode};

use crate::protocol::{Properties, Publish, QoS, RetainHandling, SubscriptionOptions};
use crate::session::{
    InflightMessage, PendingMessage, Qos2State, Session, SessionSubscription, WillMessage,
};

/// Stored retained message
#[derive(Debug, Clone, Encode, Decode)]
pub struct StoredRetainedMessage {
    pub topic: String,
    pub payload: Vec<u8>,
    pub qos: u8,
    pub properties: StoredProperties,
    /// Unix timestamp in seconds when the message was stored
    pub timestamp_secs: u64,
}

/// Stored session
#[derive(Debug, Clone, Encode, Decode)]
pub struct StoredSession {
    pub client_id: String,
    pub protocol_version: u8,
    pub session_expiry_interval: u32,
    pub keep_alive: u16,
    pub subscriptions: Vec<StoredSubscription>,
    pub pending_messages: Vec<StoredPendingMessage>,
    pub inflight_outgoing: Vec<StoredInflightMessage>,
    pub inflight_incoming: Vec<StoredInflightMessage>,
    pub will: Option<StoredWillMessage>,
    /// Unix timestamp when disconnected, if applicable
    pub disconnected_at_secs: Option<u64>,
    /// Next packet ID to use
    pub next_packet_id: u16,
}

/// Stored subscription
#[derive(Debug, Clone, Encode, Decode)]
pub struct StoredSubscription {
    pub filter: String,
    pub qos: u8,
    pub no_local: bool,
    pub retain_as_published: bool,
    pub retain_handling: u8,
    pub subscription_id: Option<u32>,
}

/// Stored pending message
#[derive(Debug, Clone, Encode, Decode)]
pub struct StoredPendingMessage {
    pub publish: StoredPublish,
    /// Unix timestamp when queued
    pub queued_at_secs: u64,
}

/// Stored inflight message
#[derive(Debug, Clone, Encode, Decode)]
pub struct StoredInflightMessage {
    pub packet_id: u16,
    pub publish: StoredPublish,
    /// 0 = None, 1 = WaitingPubRec, 2 = WaitingPubComp
    pub qos2_state: u8,
    /// Unix timestamp when sent
    pub sent_at_secs: u64,
    pub retry_count: u32,
}

/// Stored publish message
#[derive(Debug, Clone, Encode, Decode)]
pub struct StoredPublish {
    pub topic: String,
    pub payload: Vec<u8>,
    pub qos: u8,
    pub retain: bool,
    pub dup: bool,
    pub packet_id: Option<u16>,
    pub properties: StoredProperties,
}

/// Stored will message
#[derive(Debug, Clone, Encode, Decode)]
pub struct StoredWillMessage {
    pub topic: String,
    pub payload: Vec<u8>,
    pub qos: u8,
    pub retain: bool,
    pub properties: StoredProperties,
}

/// Stored MQTT v5 properties (subset relevant for persistence)
#[derive(Debug, Clone, Default, Encode, Decode)]
pub struct StoredProperties {
    pub payload_format_indicator: Option<u8>,
    pub message_expiry_interval: Option<u32>,
    pub content_type: Option<String>,
    pub response_topic: Option<String>,
    pub correlation_data: Option<Vec<u8>>,
    pub user_properties: Vec<(String, String)>,
}

/// Stored user for auth
#[derive(Debug, Clone, Encode, Decode)]
pub struct StoredUser {
    pub username: String,
    /// Always stored as argon2 hash
    pub password_hash: String,
    pub role: Option<String>,
}

/// Stored ACL role
#[derive(Debug, Clone, Encode, Decode)]
pub struct StoredRole {
    pub name: String,
    pub publish: Vec<String>,
    pub subscribe: Vec<String>,
}

// ============================================================================
// Conversion implementations
// ============================================================================

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn instant_to_unix_secs(instant: Instant) -> u64 {
    // Convert Instant to approximate Unix timestamp
    // This is an approximation since Instant doesn't map directly to wall clock
    let now = Instant::now();
    let system_now = SystemTime::now();

    if instant <= now {
        let elapsed = now.duration_since(instant);
        system_now
            .checked_sub(elapsed)
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0)
    } else {
        // Future instant (shouldn't happen but handle gracefully)
        now_unix_secs()
    }
}

fn unix_secs_to_instant(secs: u64) -> Instant {
    // Convert Unix timestamp back to Instant (approximate)
    let now = Instant::now();
    let now_unix = now_unix_secs();

    if secs <= now_unix {
        let elapsed = Duration::from_secs(now_unix - secs);
        now.checked_sub(elapsed).unwrap_or(now)
    } else {
        // Future timestamp
        now
    }
}

impl From<&Properties> for StoredProperties {
    fn from(props: &Properties) -> Self {
        Self {
            payload_format_indicator: props.payload_format_indicator,
            message_expiry_interval: props.message_expiry_interval,
            content_type: props.content_type.clone(),
            response_topic: props.response_topic.clone(),
            correlation_data: props.correlation_data.as_ref().map(|b| b.to_vec()),
            user_properties: props.user_properties.clone(),
        }
    }
}

impl From<StoredProperties> for Properties {
    fn from(stored: StoredProperties) -> Self {
        Properties {
            payload_format_indicator: stored.payload_format_indicator,
            message_expiry_interval: stored.message_expiry_interval,
            content_type: stored.content_type,
            response_topic: stored.response_topic,
            correlation_data: stored.correlation_data.map(bytes::Bytes::from),
            user_properties: stored.user_properties,
            ..Default::default()
        }
    }
}

impl From<&Publish> for StoredPublish {
    fn from(publish: &Publish) -> Self {
        Self {
            topic: publish.topic.to_string(),
            payload: publish.payload.to_vec(),
            qos: publish.qos as u8,
            retain: publish.retain,
            dup: publish.dup,
            packet_id: publish.packet_id,
            properties: StoredProperties::from(&publish.properties),
        }
    }
}

impl From<StoredPublish> for Publish {
    fn from(stored: StoredPublish) -> Self {
        Self {
            topic: Arc::from(stored.topic),
            payload: bytes::Bytes::from(stored.payload),
            qos: QoS::from_u8(stored.qos).unwrap_or_default(),
            retain: stored.retain,
            dup: stored.dup,
            packet_id: stored.packet_id,
            properties: Properties::from(stored.properties),
        }
    }
}

impl From<&WillMessage> for StoredWillMessage {
    fn from(will: &WillMessage) -> Self {
        Self {
            topic: will.topic.clone(),
            payload: will.payload.to_vec(),
            qos: will.qos as u8,
            retain: will.retain,
            properties: StoredProperties::from(&will.properties),
        }
    }
}

impl From<StoredWillMessage> for WillMessage {
    fn from(stored: StoredWillMessage) -> Self {
        Self {
            topic: stored.topic,
            payload: bytes::Bytes::from(stored.payload),
            qos: QoS::from_u8(stored.qos).unwrap_or_default(),
            retain: stored.retain,
            properties: Properties::from(stored.properties),
        }
    }
}

impl From<&SessionSubscription> for StoredSubscription {
    fn from(sub: &SessionSubscription) -> Self {
        Self {
            filter: sub.filter.clone(),
            qos: sub.options.qos as u8,
            no_local: sub.options.no_local,
            retain_as_published: sub.options.retain_as_published,
            retain_handling: sub.options.retain_handling as u8,
            subscription_id: sub.subscription_id,
        }
    }
}

impl From<StoredSubscription> for SessionSubscription {
    fn from(stored: StoredSubscription) -> Self {
        Self {
            filter: stored.filter,
            options: SubscriptionOptions {
                qos: QoS::from_u8(stored.qos).unwrap_or_default(),
                no_local: stored.no_local,
                retain_as_published: stored.retain_as_published,
                retain_handling: RetainHandling::from_u8(stored.retain_handling)
                    .unwrap_or_default(),
            },
            subscription_id: stored.subscription_id,
        }
    }
}

impl From<&PendingMessage> for StoredPendingMessage {
    fn from(pm: &PendingMessage) -> Self {
        Self {
            publish: StoredPublish::from(&pm.publish),
            queued_at_secs: instant_to_unix_secs(pm.queued_at),
        }
    }
}

impl From<StoredPendingMessage> for PendingMessage {
    fn from(stored: StoredPendingMessage) -> Self {
        Self {
            publish: Publish::from(stored.publish),
            queued_at: unix_secs_to_instant(stored.queued_at_secs),
        }
    }
}

impl StoredInflightMessage {
    /// Try to create from an InflightMessage
    /// Returns None for Cached variants (no Publish data to persist)
    pub fn try_from_inflight(im: &InflightMessage) -> Option<Self> {
        match im {
            InflightMessage::Full {
                packet_id,
                publish,
                qos2_state,
                sent_at,
                retry_count,
            } => {
                let qos2_state_code = match qos2_state {
                    None => 0,
                    Some(Qos2State::WaitingPubRec) => 1,
                    Some(Qos2State::WaitingPubComp) => 2,
                };

                Some(Self {
                    packet_id: *packet_id,
                    publish: StoredPublish::from(publish),
                    qos2_state: qos2_state_code,
                    sent_at_secs: instant_to_unix_secs(*sent_at),
                    retry_count: *retry_count,
                })
            }
            InflightMessage::Cached { .. } | InflightMessage::Raw { .. } => {
                // Cached/Raw variants don't have the original Publish data
                // They won't be persisted; on reconnect, publisher will retry
                None
            }
        }
    }
}

impl From<StoredInflightMessage> for InflightMessage {
    fn from(stored: StoredInflightMessage) -> Self {
        let qos2_state = match stored.qos2_state {
            1 => Some(Qos2State::WaitingPubRec),
            2 => Some(Qos2State::WaitingPubComp),
            _ => None,
        };

        // Always load as Full variant from storage
        InflightMessage::Full {
            packet_id: stored.packet_id,
            publish: Publish::from(stored.publish),
            qos2_state,
            sent_at: unix_secs_to_instant(stored.sent_at_secs),
            retry_count: stored.retry_count,
        }
    }
}

impl StoredSession {
    /// Create a StoredSession from a Session reference
    pub fn from_session(session: &Session) -> Self {
        Self {
            client_id: session.client_id.to_string(),
            protocol_version: session.protocol_version as u8,
            session_expiry_interval: session.session_expiry_interval,
            keep_alive: session.keep_alive,
            subscriptions: session
                .subscriptions
                .values()
                .map(StoredSubscription::from)
                .collect(),
            pending_messages: session
                .pending_messages
                .iter()
                .map(StoredPendingMessage::from)
                .collect(),
            inflight_outgoing: session
                .inflight_outgoing
                .values()
                .filter_map(StoredInflightMessage::try_from_inflight)
                .collect(),
            inflight_incoming: session
                .inflight_incoming
                .iter()
                .map(|(packet_id, publish)| StoredInflightMessage {
                    packet_id: *packet_id,
                    publish: StoredPublish::from(publish),
                    qos2_state: 0, // Incoming QoS 2 waiting for PUBREL
                    sent_at_secs: now_unix_secs(),
                    retry_count: 0,
                })
                .collect(),
            will: session.will.as_ref().map(StoredWillMessage::from),
            disconnected_at_secs: session.disconnected_at.map(instant_to_unix_secs),
            next_packet_id: 1, // Will be recalculated on restore
        }
    }
}

impl From<&crate::broker::RetainedMessage> for StoredRetainedMessage {
    fn from(rm: &crate::broker::RetainedMessage) -> Self {
        Self {
            topic: rm.topic.to_string(),
            payload: rm.payload.to_vec(),
            qos: rm.qos as u8,
            properties: StoredProperties::from(&rm.properties),
            timestamp_secs: instant_to_unix_secs(rm.timestamp),
        }
    }
}

/// Data loaded from persistence at startup
#[derive(Debug, Default)]
pub struct LoadedData {
    pub retained: Vec<(String, StoredRetainedMessage)>,
    pub sessions: Vec<(String, StoredSession)>,
    pub users: Vec<(String, StoredUser)>,
    pub roles: Vec<(String, StoredRole)>,
}

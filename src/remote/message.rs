//! Remote Message Types
//!
//! Messages exchanged between brokers for bridging and clustering.

use bytes::Bytes;

use crate::protocol::QoS;

/// A message to be forwarded to a remote broker
#[derive(Debug, Clone)]
pub enum RemoteMessage {
    /// Forward a published message
    Publish(RemotePublish),
    /// Notify of a subscription change (for clustering)
    Subscribe(RemoteSubscription),
    /// Notify of an unsubscription (for clustering)
    Unsubscribe { filter: String },
}

/// A published message to forward remotely
#[derive(Debug, Clone)]
pub struct RemotePublish {
    /// Original topic from local broker
    pub local_topic: String,
    /// Topic to publish on remote broker (may be remapped)
    pub remote_topic: String,
    /// Message payload
    pub payload: Bytes,
    /// Quality of Service level
    pub qos: QoS,
    /// Retain flag
    pub retain: bool,
}

impl RemotePublish {
    /// Create a new remote publish with same local and remote topic
    pub fn new(topic: String, payload: Bytes, qos: QoS, retain: bool) -> Self {
        Self {
            local_topic: topic.clone(),
            remote_topic: topic,
            payload,
            qos,
            retain,
        }
    }

    /// Create with topic remapping
    pub fn with_remap(
        local_topic: String,
        remote_topic: String,
        payload: Bytes,
        qos: QoS,
        retain: bool,
    ) -> Self {
        Self {
            local_topic,
            remote_topic,
            payload,
            qos,
            retain,
        }
    }
}

/// A subscription to synchronize with remote broker
#[derive(Debug, Clone)]
pub struct RemoteSubscription {
    /// Topic filter
    pub filter: String,
    /// Maximum QoS for this subscription
    pub qos: QoS,
}

impl RemoteSubscription {
    pub fn new(filter: String, qos: QoS) -> Self {
        Self { filter, qos }
    }
}

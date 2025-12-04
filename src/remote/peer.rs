//! Remote Peer Abstraction
//!
//! Trait for communication with remote brokers, used by both
//! bridging (MQTT client to external broker) and clustering
//! (custom protocol to cluster nodes).

use std::fmt;

use async_trait::async_trait;
use bytes::Bytes;

use crate::protocol::QoS;

/// Error type for remote peer operations
#[derive(Debug)]
pub enum RemoteError {
    /// Connection to remote peer failed or was lost
    ConnectionLost(String),
    /// Remote peer rejected the operation
    Rejected(String),
    /// Operation timed out
    Timeout,
    /// Message queue is full
    QueueFull,
    /// Invalid configuration
    InvalidConfig(String),
    /// Other error
    Other(String),
}

impl fmt::Display for RemoteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RemoteError::ConnectionLost(msg) => write!(f, "Connection lost: {}", msg),
            RemoteError::Rejected(msg) => write!(f, "Rejected: {}", msg),
            RemoteError::Timeout => write!(f, "Operation timed out"),
            RemoteError::QueueFull => write!(f, "Message queue full"),
            RemoteError::InvalidConfig(msg) => write!(f, "Invalid config: {}", msg),
            RemoteError::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for RemoteError {}

/// Status of a remote peer connection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemotePeerStatus {
    /// Not connected, will attempt to connect
    Disconnected,
    /// Currently connecting
    Connecting,
    /// Connected and operational
    Connected,
    /// Connection failed, backing off before retry
    Backoff,
    /// Permanently failed, will not retry
    Failed,
}

/// Trait for remote broker communication
///
/// This trait is implemented by:
/// - `BridgeClient`: MQTT client connecting to an external broker
/// - `ClusterNode` (future): Custom protocol to cluster peers
///
/// The trait provides a unified interface for message forwarding
/// that works for both bridging and clustering scenarios.
#[async_trait]
pub trait RemotePeer: Send + Sync {
    /// Get the name/identifier of this peer
    fn name(&self) -> &str;

    /// Get the current connection status
    fn status(&self) -> RemotePeerStatus;

    /// Forward a published message to this peer
    ///
    /// For bridging: publishes to the remote broker via MQTT
    /// For clustering: sends to cluster peer via internal protocol
    async fn forward_publish(
        &self,
        topic: &str,
        payload: Bytes,
        qos: QoS,
        retain: bool,
    ) -> Result<(), RemoteError>;

    /// Notify peer of a local subscription (for clustering/shared subscriptions)
    ///
    /// For bridging: may subscribe on remote broker if direction is "in" or "both"
    /// For clustering: notifies peer so it can route matching messages here
    async fn notify_subscribe(&self, filter: &str, qos: QoS) -> Result<(), RemoteError>;

    /// Notify peer of a local unsubscription
    ///
    /// For bridging: may unsubscribe from remote broker
    /// For clustering: notifies peer to stop routing those messages
    async fn notify_unsubscribe(&self, filter: &str) -> Result<(), RemoteError>;

    /// Check if this peer should receive messages for a given topic
    ///
    /// For bridging: checks topic against configured forwarding rules
    /// For clustering: checks if peer has matching subscriptions
    fn should_forward(&self, topic: &str) -> bool;

    /// Start the peer connection (called once at startup)
    async fn start(&self) -> Result<(), RemoteError>;

    /// Stop the peer connection gracefully
    async fn stop(&self) -> Result<(), RemoteError>;
}

/// A collection of remote peers for message distribution
pub struct RemotePeers {
    peers: Vec<Box<dyn RemotePeer>>,
}

impl RemotePeers {
    pub fn new() -> Self {
        Self { peers: Vec::new() }
    }

    /// Add a peer to the collection
    pub fn add(&mut self, peer: Box<dyn RemotePeer>) {
        self.peers.push(peer);
    }

    /// Forward a message to all peers that match the topic
    pub async fn forward_publish(
        &self,
        topic: &str,
        payload: Bytes,
        qos: QoS,
        retain: bool,
    ) -> Vec<(&str, Result<(), RemoteError>)> {
        let mut results = Vec::new();

        for peer in &self.peers {
            if peer.should_forward(topic) && peer.status() == RemotePeerStatus::Connected {
                let result = peer
                    .forward_publish(topic, payload.clone(), qos, retain)
                    .await;
                results.push((peer.name(), result));
            }
        }

        results
    }

    /// Notify all peers of a subscription change
    pub async fn notify_subscribe(&self, filter: &str, qos: QoS) {
        for peer in &self.peers {
            if peer.status() == RemotePeerStatus::Connected {
                let _ = peer.notify_subscribe(filter, qos).await;
            }
        }
    }

    /// Notify all peers of an unsubscription
    pub async fn notify_unsubscribe(&self, filter: &str) {
        for peer in &self.peers {
            if peer.status() == RemotePeerStatus::Connected {
                let _ = peer.notify_unsubscribe(filter).await;
            }
        }
    }

    /// Start all peer connections
    pub async fn start_all(&self) -> Vec<(&str, Result<(), RemoteError>)> {
        let mut results = Vec::new();
        for peer in &self.peers {
            let result = peer.start().await;
            results.push((peer.name(), result));
        }
        results
    }

    /// Stop all peer connections
    pub async fn stop_all(&self) -> Vec<(&str, Result<(), RemoteError>)> {
        let mut results = Vec::new();
        for peer in &self.peers {
            let result = peer.stop().await;
            results.push((peer.name(), result));
        }
        results
    }

    /// Get number of peers
    pub fn len(&self) -> usize {
        self.peers.len()
    }

    /// Check if there are no peers
    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    /// Get number of connected peers
    pub fn connected_count(&self) -> usize {
        self.peers
            .iter()
            .filter(|p| p.status() == RemotePeerStatus::Connected)
            .count()
    }
}

impl Default for RemotePeers {
    fn default() -> Self {
        Self::new()
    }
}

//! Bridge Manager
//!
//! Manages multiple bridge connections and provides a unified interface
//! for the broker to interact with all bridges.

use std::sync::Arc;

use bytes::Bytes;
use parking_lot::RwLock;
use tracing::{debug, error, info};

use crate::protocol::QoS;
use crate::remote::{RemotePeer, RemotePeerStatus};

use super::client::{BridgeClient, InboundCallback};
use crate::config::BridgeConfig;

/// Manages all bridge connections for a broker
pub struct BridgeManager {
    /// All bridge connections
    bridges: RwLock<Vec<Arc<BridgeClient>>>,
}

impl BridgeManager {
    /// Create a new bridge manager
    pub fn new() -> Self {
        Self {
            bridges: RwLock::new(Vec::new()),
        }
    }

    /// Create a bridge manager from configuration
    pub fn from_configs(configs: Vec<BridgeConfig>, inbound_callback: InboundCallback) -> Self {
        let manager = Self::new();

        for config in configs {
            if config.enabled {
                manager.add_bridge(config, inbound_callback.clone());
            }
        }

        manager
    }

    /// Add a new bridge connection
    pub fn add_bridge(&self, config: BridgeConfig, inbound_callback: InboundCallback) {
        let name = config.name.clone();
        let client = BridgeClient::new(config);
        let client = client.spawn(inbound_callback);

        info!("Bridge manager: Added bridge '{}'", name);

        self.bridges.write().push(client);
    }

    /// Forward a published message to all matching bridges
    pub async fn forward_publish(&self, topic: &str, payload: Bytes, qos: QoS, retain: bool) {
        // Collect bridges first to avoid holding lock across await
        let bridges: Vec<_> = self.bridges.read().iter().cloned().collect();

        for bridge in bridges {
            if bridge.should_forward(topic) && bridge.status() == RemotePeerStatus::Connected {
                if let Err(e) = bridge
                    .forward_publish(topic, payload.clone(), qos, retain)
                    .await
                {
                    debug!("Bridge '{}': Forward failed: {}", bridge.name(), e);
                }
            }
        }
    }

    /// Check if any bridge wants to forward a topic
    pub fn should_forward(&self, topic: &str) -> bool {
        self.bridges.read().iter().any(|b| b.should_forward(topic))
    }

    /// Get the number of bridges
    pub fn bridge_count(&self) -> usize {
        self.bridges.read().len()
    }

    /// Get the number of connected bridges
    pub fn connected_count(&self) -> usize {
        self.bridges
            .read()
            .iter()
            .filter(|b| b.status() == RemotePeerStatus::Connected)
            .count()
    }

    /// Get status of all bridges
    pub fn status(&self) -> Vec<(String, RemotePeerStatus)> {
        self.bridges
            .read()
            .iter()
            .map(|b| (b.name().to_string(), b.status()))
            .collect()
    }

    /// Start all bridges
    pub async fn start_all(&self) {
        // Collect bridges first to avoid holding lock across await
        let bridges: Vec<_> = self.bridges.read().iter().cloned().collect();
        for bridge in bridges {
            if let Err(e) = bridge.start().await {
                error!("Bridge '{}': Failed to start: {}", bridge.name(), e);
            }
        }
    }

    /// Stop all bridges
    pub async fn stop_all(&self) {
        // Collect bridges first to avoid holding lock across await
        let bridges: Vec<_> = self.bridges.read().iter().cloned().collect();
        for bridge in bridges {
            if let Err(e) = bridge.stop().await {
                error!("Bridge '{}': Failed to stop: {}", bridge.name(), e);
            }
        }
    }
}

impl Default for BridgeManager {
    fn default() -> Self {
        Self::new()
    }
}

//! MQTT Bridge Module
//!
//! Provides bridging capabilities to connect VibeMQ with external MQTT brokers.
//! Bridges can forward messages bidirectionally based on configurable topic patterns.
//!
//! # Loop Prevention
//!
//! Bridges use multiple strategies to prevent message loops:
//! - **no_local**: MQTT v5.0 subscription option that prevents receiving own messages
//! - **User Property**: Tags messages with origin broker ID to detect loops
//!
//! # Example Configuration
//!
//! ```toml
//! [[bridge]]
//! name = "cloud"
//! address = "cloud.example.com:8883"
//! protocol = "mqtts"
//! client_id = "edge-bridge-01"
//! loop_prevention = "both"  # Uses no_local AND user property
//!
//! [[bridge.forwards]]
//! local_topic = "sensors/#"
//! remote_topic = "edge/device01/sensors/#"
//! direction = "out"
//! qos = 1
//! ```

mod client;
mod manager;
mod topic_mapper;

#[cfg(test)]
mod tests;

pub use client::BridgeClient;
pub use manager::BridgeManager;
pub use topic_mapper::TopicMapper;

// Re-export config types from the config module for convenience
pub use crate::config::{
    BridgeConfig, BridgeProtocol, ForwardDirection, ForwardRule, LoopPrevention,
};

/// User property key for bridge origin tracking (loop prevention)
pub const BRIDGE_ORIGIN_PROPERTY: &str = "x-vibemq-origin";

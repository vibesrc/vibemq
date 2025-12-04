//! Cluster Module
//!
//! Provides gossip-based horizontal clustering for VibeMQ.
//!
//! # Architecture
//!
//! The cluster uses two communication channels:
//! - **Gossip (UDP via chitchat)**: Node discovery, membership, subscription state
//! - **Peer TCP**: Direct message forwarding between nodes
//!
//! # Usage
//!
//! ```toml
//! # vibemq.toml
//! [[cluster]]
//! enabled = true
//! gossip_addr = "0.0.0.0:7946"
//! peer_addr = "0.0.0.0:7947"
//! seeds = ["node1:7946", "node2:7946"]
//! ```

mod manager;
mod peer;
mod protocol;

pub use manager::ClusterManager;
pub use peer::{ClusterInboundCallback, ClusterPeer};
pub use protocol::{ClusterMessage, CLUSTER_PROTOCOL_VERSION};

// Re-export cluster config
pub use crate::config::ClusterConfig;

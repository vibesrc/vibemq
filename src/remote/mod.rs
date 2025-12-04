//! Remote Broker Communication
//!
//! Shared abstractions for bridging and clustering. This module provides
//! the core traits and types used by both bridge connections (forwarding
//! to external brokers) and cluster nodes (distributed broker instances).

mod message;
mod peer;

pub use message::{RemoteMessage, RemotePublish, RemoteSubscription};
pub use peer::{RemoteError, RemotePeer, RemotePeerStatus, RemotePeers};

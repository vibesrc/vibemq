//! VibeMQ - High-performance MQTT v3.1.1/v5.0 compliant broker
//!
//! A multi-core MQTT broker implementation with minimal dependencies,
//! designed for maximum performance and full protocol compliance.

pub mod acl;
pub mod auth;
pub mod bridge;
pub mod broker;
pub mod cluster;
pub mod codec;
pub mod config;
pub mod hooks;
pub mod metrics;
#[cfg(feature = "pprof")]
pub mod profiling;
pub mod protocol;
pub mod remote;
pub mod session;
pub mod topic;
pub mod transport;

pub use acl::AclProvider;
pub use auth::AuthProvider;
pub use bridge::{BridgeClient, BridgeConfig, BridgeManager};
pub use broker::Broker;
pub use cluster::{ClusterConfig, ClusterManager};
pub use config::Config;
pub use hooks::{CompositeHooks, DefaultHooks, Hooks};
pub use metrics::{Metrics, MetricsServer};
pub use protocol::{ProtocolVersion, QoS};
pub use remote::{RemoteError, RemotePeer, RemotePeerStatus};

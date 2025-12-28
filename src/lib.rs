//! VibeMQ - High-performance MQTT v3.1.1/v5.0 compliant broker
//!
//! A multi-core MQTT broker implementation with minimal dependencies,
//! designed for maximum performance and full protocol compliance.

pub mod acl;
pub mod auth;
pub mod bridge;
pub mod broker;
pub mod buffer_pool;
pub mod cluster;
pub mod codec;
pub mod config;
pub mod flapping;
pub mod hooks;
pub mod metrics;
pub mod persistence;
#[cfg(feature = "pprof")]
pub mod profiling;
pub mod protocol;
pub mod proxy;
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
pub use flapping::{ConnectionLimitConfig, FlappingConfig, FlappingDetector};
pub use hooks::{CompositeHooks, DefaultHooks, Hooks};
pub use metrics::{Metrics, MetricsServer};
pub use persistence::{FjallBackend, PersistenceManager, StorageBackend};
pub use protocol::{ProtocolVersion, QoS};
pub use remote::{RemoteError, RemotePeer, RemotePeerStatus};

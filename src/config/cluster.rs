//! Cluster Configuration
//!
//! Configuration types for gossip-based horizontal clustering.

use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

use serde::Deserialize;

/// Cluster configuration for gossip-based horizontal scaling
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ClusterConfig {
    /// Whether clustering is enabled
    pub enabled: bool,

    /// Node identifier (auto-generated from hostname if not set)
    pub node_id: Option<String>,

    /// Address for gossip protocol (chitchat) to bind to
    /// Default: 0.0.0.0:7946
    #[serde(default = "default_gossip_addr")]
    pub gossip_addr: SocketAddr,

    /// Advertise address for gossip protocol (what peers use to reach us)
    /// If not set, resolved from hostname or falls back to gossip_addr
    pub gossip_advertise_addr: Option<SocketAddr>,

    /// Address for peer-to-peer message relay to bind to
    /// Default: 0.0.0.0:7947
    #[serde(default = "default_peer_addr")]
    pub peer_addr: SocketAddr,

    /// Advertise address for peer connections (what peers use to reach us)
    /// If not set, resolved from hostname or falls back to peer_addr
    pub peer_advertise_addr: Option<SocketAddr>,

    /// Seed nodes for cluster discovery
    /// Format: "host:port" (gossip port)
    #[serde(default)]
    pub seeds: Vec<String>,

    /// Gossip interval in seconds
    /// Default: 1
    #[serde(default = "default_gossip_interval")]
    pub gossip_interval: u64,

    /// Node failure detection timeout in seconds
    /// Default: 5
    #[serde(default = "default_failure_timeout")]
    pub failure_timeout: u64,

    /// Dead node grace period in seconds before removal
    /// Default: 30
    #[serde(default = "default_dead_node_grace_period")]
    pub dead_node_grace_period: u64,
}

fn default_gossip_addr() -> SocketAddr {
    "0.0.0.0:7946".parse().unwrap()
}

fn default_peer_addr() -> SocketAddr {
    "0.0.0.0:7947".parse().unwrap()
}

fn default_gossip_interval() -> u64 {
    1
}

fn default_failure_timeout() -> u64 {
    5
}

fn default_dead_node_grace_period() -> u64 {
    30
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            node_id: None,
            gossip_addr: default_gossip_addr(),
            gossip_advertise_addr: None,
            peer_addr: default_peer_addr(),
            peer_advertise_addr: None,
            seeds: Vec::new(),
            gossip_interval: default_gossip_interval(),
            failure_timeout: default_failure_timeout(),
            dead_node_grace_period: default_dead_node_grace_period(),
        }
    }
}

impl ClusterConfig {
    /// Get the node ID, generating from hostname if not set
    pub fn get_node_id(&self) -> String {
        self.node_id.clone().unwrap_or_else(|| {
            hostname::get()
                .map(|h| h.to_string_lossy().to_string())
                .unwrap_or_else(|_| format!("node-{}", rand_id()))
        })
    }

    /// Get the gossip advertise address (what peers use to reach us)
    /// Priority: explicit config > resolved hostname > bind address
    pub fn get_gossip_advertise_addr(&self) -> SocketAddr {
        if let Some(addr) = self.gossip_advertise_addr {
            return addr;
        }

        // Try to resolve our hostname to get the real IP
        if let Some(ip) = resolve_local_ip() {
            return SocketAddr::new(ip, self.gossip_addr.port());
        }

        // Fallback to bind address
        self.gossip_addr
    }

    /// Get the peer advertise address (what peers use to reach us)
    /// Priority: explicit config > resolved hostname > bind address
    pub fn get_peer_advertise_addr(&self) -> SocketAddr {
        if let Some(addr) = self.peer_advertise_addr {
            return addr;
        }

        // Try to resolve our hostname to get the real IP
        if let Some(ip) = resolve_local_ip() {
            return SocketAddr::new(ip, self.peer_addr.port());
        }

        // Fallback to bind address
        self.peer_addr
    }

    /// Get gossip interval as Duration
    pub fn gossip_interval_duration(&self) -> Duration {
        Duration::from_secs(self.gossip_interval)
    }

    /// Get failure timeout as Duration
    pub fn failure_timeout_duration(&self) -> Duration {
        Duration::from_secs(self.failure_timeout)
    }

    /// Get dead node grace period as Duration
    pub fn dead_node_grace_period_duration(&self) -> Duration {
        Duration::from_secs(self.dead_node_grace_period)
    }
}

/// Resolve the local machine's IP address by resolving the hostname
fn resolve_local_ip() -> Option<IpAddr> {
    let hostname = hostname::get().ok()?;
    let hostname_str = hostname.to_string_lossy();

    // Try to resolve hostname:0 to get the IP
    let addr_str = format!("{}:0", hostname_str);
    addr_str
        .to_socket_addrs()
        .ok()?
        .find(|addr| addr.is_ipv4()) // Prefer IPv4
        .map(|addr| addr.ip())
}

/// Generate a random ID for node identification
fn rand_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}", nanos & 0xFFFFFFFF)
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ClusterConfig::default();
        assert!(!config.enabled);
        assert!(config.node_id.is_none());
        assert_eq!(config.gossip_addr, "0.0.0.0:7946".parse().unwrap());
        assert_eq!(config.peer_addr, "0.0.0.0:7947".parse().unwrap());
        assert!(config.seeds.is_empty());
    }

    #[test]
    fn test_get_node_id_with_explicit() {
        let mut config = ClusterConfig::default();
        config.node_id = Some("my-node".to_string());
        assert_eq!(config.get_node_id(), "my-node");
    }

    #[test]
    fn test_get_node_id_auto_generated() {
        let config = ClusterConfig::default();
        let id = config.get_node_id();
        assert!(!id.is_empty());
    }

    #[test]
    fn test_duration_conversions() {
        let mut config = ClusterConfig::default();
        config.gossip_interval = 2;
        config.failure_timeout = 10;
        config.dead_node_grace_period = 60;

        assert_eq!(config.gossip_interval_duration(), Duration::from_secs(2));
        assert_eq!(config.failure_timeout_duration(), Duration::from_secs(10));
        assert_eq!(
            config.dead_node_grace_period_duration(),
            Duration::from_secs(60)
        );
    }
}

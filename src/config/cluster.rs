//! Cluster Configuration
//!
//! Configuration types for gossip-based horizontal clustering.

use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

use serde::Deserialize;

use super::ProxyProtocolConfig;

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

    /// Gossip interval (e.g., "1s", "500ms")
    /// Default: 1s
    #[serde(default = "default_gossip_interval", with = "humantime_serde")]
    pub gossip_interval: Duration,

    /// Node failure detection timeout (e.g., "5s", "10s")
    /// Default: 5s
    #[serde(default = "default_failure_timeout", with = "humantime_serde")]
    pub failure_timeout: Duration,

    /// Dead node grace period before removal (e.g., "30s", "1m")
    /// Default: 30s
    #[serde(default = "default_dead_node_grace_period", with = "humantime_serde")]
    pub dead_node_grace_period: Duration,

    /// PROXY protocol configuration for peer listener
    #[serde(default)]
    pub proxy_protocol: ProxyProtocolConfig,
}

fn default_gossip_addr() -> SocketAddr {
    "0.0.0.0:7946".parse().unwrap()
}

fn default_peer_addr() -> SocketAddr {
    "0.0.0.0:7947".parse().unwrap()
}

fn default_gossip_interval() -> Duration {
    Duration::from_secs(1)
}

fn default_failure_timeout() -> Duration {
    Duration::from_secs(5)
}

fn default_dead_node_grace_period() -> Duration {
    Duration::from_secs(30)
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
            gossip_interval: Duration::from_secs(1),
            failure_timeout: Duration::from_secs(5),
            dead_node_grace_period: Duration::from_secs(30),
            proxy_protocol: ProxyProtocolConfig::default(),
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
    fn test_duration_defaults() {
        let config = ClusterConfig::default();
        assert_eq!(config.gossip_interval, Duration::from_secs(1));
        assert_eq!(config.failure_timeout, Duration::from_secs(5));
        assert_eq!(config.dead_node_grace_period, Duration::from_secs(30));
    }
}

//! Metrics configuration

use serde::Deserialize;
use std::net::SocketAddr;

/// Metrics configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MetricsConfig {
    /// Whether metrics are enabled
    pub enabled: bool,
    /// HTTP bind address for metrics endpoint
    pub bind: SocketAddr,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: "0.0.0.0:9090".parse().unwrap(),
        }
    }
}

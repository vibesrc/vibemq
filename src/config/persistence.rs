//! Persistence configuration.

use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

/// Backend type for persistence
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendType {
    /// Fjall (local LSM-tree storage)
    #[default]
    Fjall,
    // Future: Redis, Postgres, etc.
}

fn default_flush_interval() -> Duration {
    Duration::from_millis(100)
}

/// Persistence configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PersistenceConfig {
    /// Enable persistence
    pub enabled: bool,

    /// Backend type
    pub backend: BackendType,

    /// Data directory path (for fjall)
    pub path: PathBuf,

    /// Flush interval (e.g., "100ms", "1s")
    #[serde(default = "default_flush_interval", with = "humantime_serde")]
    pub flush_interval: Duration,

    /// Maximum batch size before forced flush
    pub max_batch_size: usize,
}

impl Default for PersistenceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: BackendType::Fjall,
            path: PathBuf::from("./data"),
            flush_interval: Duration::from_millis(100),
            max_batch_size: 100,
        }
    }
}

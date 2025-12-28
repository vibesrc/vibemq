//! Persistence configuration.

use std::path::PathBuf;

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

    /// Flush interval in milliseconds
    pub flush_interval_ms: u64,

    /// Maximum batch size before forced flush
    pub max_batch_size: usize,
}

impl Default for PersistenceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: BackendType::Fjall,
            path: PathBuf::from("./data"),
            flush_interval_ms: 100,
            max_batch_size: 100,
        }
    }
}

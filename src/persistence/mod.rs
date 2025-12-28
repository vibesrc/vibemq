//! Persistence module for VibeMQ.
//!
//! Provides durable storage for:
//! - Retained messages
//! - Sessions (with inflight QoS 1/2 messages)
//! - Users and ACL roles (for future HTTP API)
//!
//! Uses a trait-based design allowing different backends:
//! - `FjallBackend` (default) - Local LSM-tree storage
//! - Future: Redis, PostgreSQL, etc.

mod backend;
mod error;
mod fjall;
mod models;

pub use backend::{PersistenceOp, StorageBackend};
pub use error::{PersistenceError, Result};
pub use fjall::FjallBackend;
pub use models::{
    LoadedData, StoredInflightMessage, StoredPendingMessage, StoredProperties, StoredPublish,
    StoredRetainedMessage, StoredRole, StoredSession, StoredSubscription, StoredUser,
    StoredWillMessage,
};

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// Persistence manager that handles background writes
pub struct PersistenceManager {
    backend: Arc<dyn StorageBackend>,
    tx: mpsc::Sender<PersistenceOp>,
    shutdown_tx: mpsc::Sender<()>,
}

impl PersistenceManager {
    /// Create a new persistence manager with the given backend
    ///
    /// This spawns a background task that batches and commits writes.
    pub fn new(
        backend: Arc<dyn StorageBackend>,
        flush_interval: Duration,
        max_batch_size: usize,
    ) -> Self {
        let (tx, rx) = mpsc::channel(10_000);
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

        // Spawn background writer task
        let backend_clone = backend.clone();
        tokio::spawn(Self::writer_loop(
            backend_clone,
            rx,
            shutdown_rx,
            flush_interval,
            max_batch_size,
        ));

        Self {
            backend,
            tx,
            shutdown_tx,
        }
    }

    /// Fire-and-forget write operation (non-blocking for hot path)
    ///
    /// If the channel is full, the operation is dropped (backpressure).
    pub fn write(&self, op: PersistenceOp) {
        if let Err(e) = self.tx.try_send(op) {
            warn!("Persistence channel full, dropping operation: {:?}", e);
        }
    }

    /// Load all data at startup
    pub async fn load_all(&self) -> Result<LoadedData> {
        self.backend.load_all().await
    }

    /// Gracefully shutdown the persistence manager
    ///
    /// This flushes all pending writes and closes the backend.
    pub async fn shutdown(&self) -> Result<()> {
        info!("Shutting down persistence manager");

        // Signal writer task to stop
        let _ = self.shutdown_tx.send(()).await;

        // Give the writer task time to flush
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Final flush
        self.backend.flush().await?;
        self.backend.close().await?;

        info!("Persistence manager shutdown complete");
        Ok(())
    }

    /// Background writer loop that batches and commits writes
    async fn writer_loop(
        backend: Arc<dyn StorageBackend>,
        mut rx: mpsc::Receiver<PersistenceOp>,
        mut shutdown_rx: mpsc::Receiver<()>,
        flush_interval: Duration,
        max_batch_size: usize,
    ) {
        let mut batch = Vec::with_capacity(max_batch_size);
        let mut interval = tokio::time::interval(flush_interval);

        loop {
            tokio::select! {
                // Receive operations
                op = rx.recv() => {
                    match op {
                        Some(op) => {
                            batch.push(op);

                            // Flush immediately if batch is large
                            if batch.len() >= max_batch_size {
                                if let Err(e) = backend.batch_write(std::mem::take(&mut batch)).await {
                                    error!("Failed to write batch: {}", e);
                                } else {
                                    debug!("Flushed {} operations (max batch)", batch.capacity());
                                }
                            }
                        }
                        None => {
                            // Channel closed, flush remaining and exit
                            if !batch.is_empty() {
                                if let Err(e) = backend.batch_write(std::mem::take(&mut batch)).await {
                                    error!("Failed to write final batch: {}", e);
                                }
                            }
                            break;
                        }
                    }
                }

                // Periodic flush
                _ = interval.tick() => {
                    if !batch.is_empty() {
                        let count = batch.len();
                        if let Err(e) = backend.batch_write(std::mem::take(&mut batch)).await {
                            error!("Failed to write batch: {}", e);
                        } else {
                            debug!("Flushed {} operations (interval)", count);
                        }
                    }
                }

                // Shutdown signal
                _ = shutdown_rx.recv() => {
                    info!("Persistence writer received shutdown signal");
                    // Flush remaining operations
                    if !batch.is_empty() {
                        let count = batch.len();
                        if let Err(e) = backend.batch_write(std::mem::take(&mut batch)).await {
                            error!("Failed to write final batch on shutdown: {}", e);
                        } else {
                            info!("Flushed {} operations on shutdown", count);
                        }
                    }
                    break;
                }
            }
        }

        info!("Persistence writer loop exited");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    #[tokio::test]
    async fn test_fjall_backend_basic_operations() {
        let temp_dir = tempfile::tempdir().unwrap();
        let backend = FjallBackend::open(temp_dir.path()).unwrap();

        // Test retained message
        let message = StoredRetainedMessage {
            topic: "test/topic".to_string(),
            payload: vec![1, 2, 3],
            qos: 1,
            properties: StoredProperties::default(),
            timestamp_secs: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };

        backend.set_retained("test/topic", &message).await.unwrap();

        let retrieved = backend.get_retained("test/topic").await.unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.topic, "test/topic");
        assert_eq!(retrieved.payload, vec![1, 2, 3]);

        backend.delete_retained("test/topic").await.unwrap();
        assert!(backend.get_retained("test/topic").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_fjall_backend_batch_write() {
        let temp_dir = tempfile::tempdir().unwrap();
        let backend = FjallBackend::open(temp_dir.path()).unwrap();

        let now_secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let ops = vec![
            PersistenceOp::SetRetained {
                topic: "topic1".to_string(),
                message: StoredRetainedMessage {
                    topic: "topic1".to_string(),
                    payload: vec![1],
                    qos: 0,
                    properties: StoredProperties::default(),
                    timestamp_secs: now_secs,
                },
            },
            PersistenceOp::SetRetained {
                topic: "topic2".to_string(),
                message: StoredRetainedMessage {
                    topic: "topic2".to_string(),
                    payload: vec![2],
                    qos: 1,
                    properties: StoredProperties::default(),
                    timestamp_secs: now_secs,
                },
            },
        ];

        backend.batch_write(ops).await.unwrap();

        let retained = backend.list_retained().await.unwrap();
        assert_eq!(retained.len(), 2);
    }
}

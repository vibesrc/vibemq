//! Storage backend trait for persistence.
//!
//! This trait defines the interface for persistence backends,
//! allowing different implementations (fjall, Redis, PostgreSQL, etc.)

use async_trait::async_trait;

use super::error::Result;
use super::models::{
    LoadedData, StoredRetainedMessage, StoredRole, StoredSession, StoredUser,
};

/// Persistence operation for batch writes
#[derive(Debug, Clone)]
pub enum PersistenceOp {
    /// Set a retained message
    SetRetained {
        topic: String,
        message: StoredRetainedMessage,
    },
    /// Delete a retained message
    DeleteRetained { topic: String },
    /// Set a session
    SetSession {
        client_id: String,
        session: StoredSession,
    },
    /// Delete a session
    DeleteSession { client_id: String },
    /// Set a user
    SetUser { username: String, user: StoredUser },
    /// Delete a user
    DeleteUser { username: String },
    /// Set a role
    SetRole { name: String, role: StoredRole },
    /// Delete a role
    DeleteRole { name: String },
}

/// Storage backend trait for persistence
#[async_trait]
pub trait StorageBackend: Send + Sync {
    // ========================================================================
    // Retained messages
    // ========================================================================

    /// Get a retained message by topic
    async fn get_retained(&self, topic: &str) -> Result<Option<StoredRetainedMessage>>;

    /// Set a retained message
    async fn set_retained(&self, topic: &str, message: &StoredRetainedMessage) -> Result<()>;

    /// Delete a retained message
    async fn delete_retained(&self, topic: &str) -> Result<()>;

    /// List all retained messages
    async fn list_retained(&self) -> Result<Vec<(String, StoredRetainedMessage)>>;

    // ========================================================================
    // Sessions
    // ========================================================================

    /// Get a session by client ID
    async fn get_session(&self, client_id: &str) -> Result<Option<StoredSession>>;

    /// Set a session
    async fn set_session(&self, client_id: &str, session: &StoredSession) -> Result<()>;

    /// Delete a session
    async fn delete_session(&self, client_id: &str) -> Result<()>;

    /// List all sessions
    async fn list_sessions(&self) -> Result<Vec<(String, StoredSession)>>;

    // ========================================================================
    // Users (for future HTTP API)
    // ========================================================================

    /// Get a user by username
    async fn get_user(&self, username: &str) -> Result<Option<StoredUser>>;

    /// Set a user
    async fn set_user(&self, username: &str, user: &StoredUser) -> Result<()>;

    /// Delete a user
    async fn delete_user(&self, username: &str) -> Result<()>;

    /// List all users
    async fn list_users(&self) -> Result<Vec<(String, StoredUser)>>;

    // ========================================================================
    // ACL roles (for future HTTP API)
    // ========================================================================

    /// Get a role by name
    async fn get_role(&self, name: &str) -> Result<Option<StoredRole>>;

    /// Set a role
    async fn set_role(&self, name: &str, role: &StoredRole) -> Result<()>;

    /// Delete a role
    async fn delete_role(&self, name: &str) -> Result<()>;

    /// List all roles
    async fn list_roles(&self) -> Result<Vec<(String, StoredRole)>>;

    // ========================================================================
    // Batch operations
    // ========================================================================

    /// Execute a batch of operations atomically
    async fn batch_write(&self, ops: Vec<PersistenceOp>) -> Result<()>;

    // ========================================================================
    // Lifecycle
    // ========================================================================

    /// Flush all pending writes to disk
    async fn flush(&self) -> Result<()>;

    /// Close the backend (flush and release resources)
    async fn close(&self) -> Result<()>;

    /// Load all data at startup
    async fn load_all(&self) -> Result<LoadedData> {
        let retained = self.list_retained().await?;
        let sessions = self.list_sessions().await?;
        let users = self.list_users().await?;
        let roles = self.list_roles().await?;

        Ok(LoadedData {
            retained,
            sessions,
            users,
            roles,
        })
    }
}

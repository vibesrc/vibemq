//! Fjall-based storage backend implementation.
//!
//! Uses fjall (an LSM-tree based embedded database) for local persistence.

use std::path::Path;

use async_trait::async_trait;
use fjall::{Config, Keyspace, PartitionCreateOptions, PartitionHandle, PersistMode};

use super::backend::{PersistenceOp, StorageBackend};
use super::error::{PersistenceError, Result};
use super::models::{StoredRetainedMessage, StoredRole, StoredSession, StoredUser};

/// Fjall-based storage backend
pub struct FjallBackend {
    keyspace: Keyspace,
    retained: PartitionHandle,
    sessions: PartitionHandle,
    users: PartitionHandle,
    roles: PartitionHandle,
}

impl FjallBackend {
    /// Open a fjall backend at the given path
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let keyspace = Config::new(path).open()?;

        let retained = keyspace.open_partition("retained", PartitionCreateOptions::default())?;
        let sessions = keyspace.open_partition("sessions", PartitionCreateOptions::default())?;
        let users = keyspace.open_partition("users", PartitionCreateOptions::default())?;
        let roles = keyspace.open_partition("roles", PartitionCreateOptions::default())?;

        Ok(Self {
            keyspace,
            retained,
            sessions,
            users,
            roles,
        })
    }

    /// Serialize a value using bincode
    fn serialize<T: bincode::Encode>(value: &T) -> Result<Vec<u8>> {
        bincode::encode_to_vec(value, bincode::config::standard()).map_err(PersistenceError::from)
    }

    /// Deserialize a value using bincode
    fn deserialize<T: bincode::Decode<()>>(bytes: &[u8]) -> Result<T> {
        bincode::decode_from_slice(bytes, bincode::config::standard())
            .map(|(value, _)| value)
            .map_err(PersistenceError::from)
    }
}

#[async_trait]
impl StorageBackend for FjallBackend {
    // ========================================================================
    // Retained messages
    // ========================================================================

    async fn get_retained(&self, topic: &str) -> Result<Option<StoredRetainedMessage>> {
        match self.retained.get(topic)? {
            Some(bytes) => Ok(Some(Self::deserialize(&bytes)?)),
            None => Ok(None),
        }
    }

    async fn set_retained(&self, topic: &str, message: &StoredRetainedMessage) -> Result<()> {
        let bytes = Self::serialize(message)?;
        self.retained.insert(topic, bytes)?;
        Ok(())
    }

    async fn delete_retained(&self, topic: &str) -> Result<()> {
        self.retained.remove(topic)?;
        Ok(())
    }

    async fn list_retained(&self) -> Result<Vec<(String, StoredRetainedMessage)>> {
        let mut result = Vec::new();
        for item in self.retained.iter() {
            let (key, value) = item?;
            let topic = String::from_utf8_lossy(&key).to_string();
            let message: StoredRetainedMessage = Self::deserialize(&value)?;
            result.push((topic, message));
        }
        Ok(result)
    }

    // ========================================================================
    // Sessions
    // ========================================================================

    async fn get_session(&self, client_id: &str) -> Result<Option<StoredSession>> {
        match self.sessions.get(client_id)? {
            Some(bytes) => Ok(Some(Self::deserialize(&bytes)?)),
            None => Ok(None),
        }
    }

    async fn set_session(&self, client_id: &str, session: &StoredSession) -> Result<()> {
        let bytes = Self::serialize(session)?;
        self.sessions.insert(client_id, bytes)?;
        Ok(())
    }

    async fn delete_session(&self, client_id: &str) -> Result<()> {
        self.sessions.remove(client_id)?;
        Ok(())
    }

    async fn list_sessions(&self) -> Result<Vec<(String, StoredSession)>> {
        let mut result = Vec::new();
        for item in self.sessions.iter() {
            let (key, value) = item?;
            let client_id = String::from_utf8_lossy(&key).to_string();
            let session: StoredSession = Self::deserialize(&value)?;
            result.push((client_id, session));
        }
        Ok(result)
    }

    // ========================================================================
    // Users
    // ========================================================================

    async fn get_user(&self, username: &str) -> Result<Option<StoredUser>> {
        match self.users.get(username)? {
            Some(bytes) => Ok(Some(Self::deserialize(&bytes)?)),
            None => Ok(None),
        }
    }

    async fn set_user(&self, username: &str, user: &StoredUser) -> Result<()> {
        let bytes = Self::serialize(user)?;
        self.users.insert(username, bytes)?;
        Ok(())
    }

    async fn delete_user(&self, username: &str) -> Result<()> {
        self.users.remove(username)?;
        Ok(())
    }

    async fn list_users(&self) -> Result<Vec<(String, StoredUser)>> {
        let mut result = Vec::new();
        for item in self.users.iter() {
            let (key, value) = item?;
            let username = String::from_utf8_lossy(&key).to_string();
            let user: StoredUser = Self::deserialize(&value)?;
            result.push((username, user));
        }
        Ok(result)
    }

    // ========================================================================
    // Roles
    // ========================================================================

    async fn get_role(&self, name: &str) -> Result<Option<StoredRole>> {
        match self.roles.get(name)? {
            Some(bytes) => Ok(Some(Self::deserialize(&bytes)?)),
            None => Ok(None),
        }
    }

    async fn set_role(&self, name: &str, role: &StoredRole) -> Result<()> {
        let bytes = Self::serialize(role)?;
        self.roles.insert(name, bytes)?;
        Ok(())
    }

    async fn delete_role(&self, name: &str) -> Result<()> {
        self.roles.remove(name)?;
        Ok(())
    }

    async fn list_roles(&self) -> Result<Vec<(String, StoredRole)>> {
        let mut result = Vec::new();
        for item in self.roles.iter() {
            let (key, value) = item?;
            let name = String::from_utf8_lossy(&key).to_string();
            let role: StoredRole = Self::deserialize(&value)?;
            result.push((name, role));
        }
        Ok(result)
    }

    // ========================================================================
    // Batch operations
    // ========================================================================

    async fn batch_write(&self, ops: Vec<PersistenceOp>) -> Result<()> {
        let mut batch = self.keyspace.batch();

        for op in ops {
            match op {
                PersistenceOp::SetRetained { topic, message } => {
                    let bytes = Self::serialize(&message)?;
                    batch.insert(&self.retained, topic, bytes);
                }
                PersistenceOp::DeleteRetained { topic } => {
                    batch.remove(&self.retained, topic);
                }
                PersistenceOp::SetSession { client_id, session } => {
                    let bytes = Self::serialize(&session)?;
                    batch.insert(&self.sessions, client_id, bytes);
                }
                PersistenceOp::DeleteSession { client_id } => {
                    batch.remove(&self.sessions, client_id);
                }
                PersistenceOp::SetUser { username, user } => {
                    let bytes = Self::serialize(&user)?;
                    batch.insert(&self.users, username, bytes);
                }
                PersistenceOp::DeleteUser { username } => {
                    batch.remove(&self.users, username);
                }
                PersistenceOp::SetRole { name, role } => {
                    let bytes = Self::serialize(&role)?;
                    batch.insert(&self.roles, name, bytes);
                }
                PersistenceOp::DeleteRole { name } => {
                    batch.remove(&self.roles, name);
                }
            }
        }

        batch.commit()?;
        Ok(())
    }

    // ========================================================================
    // Lifecycle
    // ========================================================================

    async fn flush(&self) -> Result<()> {
        self.keyspace.persist(PersistMode::SyncAll)?;
        Ok(())
    }

    async fn close(&self) -> Result<()> {
        // Flush before closing
        self.flush().await?;
        // fjall handles cleanup on drop
        Ok(())
    }
}

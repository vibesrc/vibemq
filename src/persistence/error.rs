//! Persistence error types.

use std::fmt;

/// Errors that can occur during persistence operations.
#[derive(Debug)]
pub enum PersistenceError {
    /// IO error
    Io(std::io::Error),
    /// Serialization error
    Serialize(String),
    /// Deserialization error
    Deserialize(String),
    /// Storage backend error
    Storage(String),
    /// Data corruption detected
    Corruption(String),
}

impl fmt::Display for PersistenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {}", e),
            Self::Serialize(e) => write!(f, "serialization error: {}", e),
            Self::Deserialize(e) => write!(f, "deserialization error: {}", e),
            Self::Storage(e) => write!(f, "storage error: {}", e),
            Self::Corruption(e) => write!(f, "data corruption: {}", e),
        }
    }
}

impl std::error::Error for PersistenceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for PersistenceError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<fjall::Error> for PersistenceError {
    fn from(err: fjall::Error) -> Self {
        Self::Storage(err.to_string())
    }
}

impl From<bincode::error::EncodeError> for PersistenceError {
    fn from(err: bincode::error::EncodeError) -> Self {
        Self::Serialize(err.to_string())
    }
}

impl From<bincode::error::DecodeError> for PersistenceError {
    fn from(err: bincode::error::DecodeError) -> Self {
        Self::Deserialize(err.to_string())
    }
}

/// Result type for persistence operations.
pub type Result<T> = std::result::Result<T, PersistenceError>;

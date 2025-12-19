//! Authentication Module
//!
//! Provides username/password authentication with support for:
//! - Plaintext passwords (for development/testing)
//! - Argon2 password hashes (recommended for production)

use std::collections::HashMap;
use std::sync::Arc;

use argon2::{Argon2, PasswordHash, PasswordVerifier};
use async_trait::async_trait;
use parking_lot::RwLock;

use crate::config::AuthConfig;
use crate::hooks::{HookResult, Hooks};

#[cfg(test)]
mod tests;

/// Authentication provider
pub struct AuthProvider {
    /// Whether auth is enabled
    enabled: bool,
    /// Allow anonymous connections
    allow_anonymous: bool,
    /// User credentials map (username -> UserEntry)
    users: HashMap<String, UserEntry>,
    /// Connected client usernames (for ACL lookups)
    client_usernames: Arc<RwLock<HashMap<String, Option<String>>>>,
}

/// Credential storage type
enum Credential {
    /// Plaintext password (for development/testing)
    Plaintext(String),
    /// Argon2 password hash (for production)
    Argon2Hash(String),
}

/// Internal user entry
struct UserEntry {
    /// User credential (plaintext or hash)
    credential: Credential,
    /// ACL role (if any)
    role: Option<String>,
}

impl AuthProvider {
    /// Create a new auth provider from configuration
    pub fn new(config: &AuthConfig) -> Self {
        let mut users = HashMap::new();

        for user in &config.users {
            let credential = if let Some(ref hash) = user.password_hash {
                Credential::Argon2Hash(hash.clone())
            } else if let Some(ref pwd) = user.password {
                Credential::Plaintext(pwd.clone())
            } else {
                // This shouldn't happen if config validation is done
                continue;
            };

            users.insert(
                user.username.clone(),
                UserEntry {
                    credential,
                    role: user.role.clone(),
                },
            );
        }

        Self {
            enabled: config.enabled,
            allow_anonymous: config.allow_anonymous,
            users,
            client_usernames: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Check if auth is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Get the ACL role for a username
    pub fn get_user_role(&self, username: &str) -> Option<&str> {
        self.users.get(username).and_then(|u| u.role.as_deref())
    }

    /// Get the username for a connected client
    pub fn get_client_username(&self, client_id: &str) -> Option<String> {
        self.client_usernames
            .read()
            .get(client_id)
            .and_then(|u| u.clone())
    }

    /// Verify a password against stored credential
    fn verify_password(&self, password: &[u8], credential: &Credential) -> bool {
        match credential {
            Credential::Plaintext(stored) => {
                // Compare plaintext password
                match std::str::from_utf8(password) {
                    Ok(pwd) => pwd == stored,
                    Err(_) => false,
                }
            }
            Credential::Argon2Hash(hash) => {
                // Verify against argon2 hash
                let Ok(parsed_hash) = PasswordHash::new(hash) else {
                    return false;
                };
                Argon2::default()
                    .verify_password(password, &parsed_hash)
                    .is_ok()
            }
        }
    }

    /// Store client username mapping
    fn store_client_username(&self, client_id: &str, username: Option<&str>) {
        self.client_usernames
            .write()
            .insert(client_id.to_string(), username.map(|s| s.to_string()));
    }

    /// Remove client username mapping
    pub fn remove_client_username(&self, client_id: &str) {
        self.client_usernames.write().remove(client_id);
    }
}

#[async_trait]
impl Hooks for AuthProvider {
    async fn on_authenticate(
        &self,
        client_id: &str,
        username: Option<&str>,
        password: Option<&[u8]>,
    ) -> HookResult<bool> {
        // If auth is disabled, allow all
        if !self.enabled {
            self.store_client_username(client_id, username);
            return Ok(true);
        }

        // Check for anonymous connection
        if username.is_none() {
            if self.allow_anonymous {
                self.store_client_username(client_id, None);
                return Ok(true);
            } else {
                return Ok(false);
            }
        }

        let username = username.unwrap();
        let password = password.unwrap_or(&[]);

        // Look up user
        let user = match self.users.get(username) {
            Some(u) => u,
            None => return Ok(false),
        };

        // Verify password
        if self.verify_password(password, &user.credential) {
            self.store_client_username(client_id, Some(username));
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn on_client_disconnected(&self, client_id: &str, _graceful: bool) {
        self.remove_client_username(client_id);
    }
}

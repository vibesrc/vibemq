//! Authentication Module
//!
//! Provides username/password authentication with plaintext password storage.

use std::collections::HashMap;
use std::sync::Arc;

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

/// Internal user entry
struct UserEntry {
    /// Password (plaintext)
    password: String,
    /// ACL role (if any)
    role: Option<String>,
}

impl AuthProvider {
    /// Create a new auth provider from configuration
    pub fn new(config: &AuthConfig) -> Self {
        let mut users = HashMap::new();

        for user in &config.users {
            users.insert(
                user.username.clone(),
                UserEntry {
                    password: user.password.clone(),
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

    /// Verify a password against stored password
    fn verify_password(&self, password: &[u8], stored: &str) -> bool {
        // Convert password bytes to string and compare
        match std::str::from_utf8(password) {
            Ok(pwd) => pwd == stored,
            Err(_) => false,
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
        if self.verify_password(password, &user.password) {
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

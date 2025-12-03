//! ACL (Access Control List) Module
//!
//! Provides topic-based authorization with support for:
//! - MQTT wildcards (# and +)
//! - Variable substitution (%c = client_id, %u = username)
//! - Role-based permissions

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::auth::AuthProvider;
use crate::config::AclConfig;
use crate::hooks::{HookResult, Hooks};
use crate::protocol::QoS;

#[cfg(test)]
mod tests;

/// ACL provider
pub struct AclProvider {
    /// Whether ACL is enabled
    enabled: bool,
    /// Role definitions (name -> role)
    roles: HashMap<String, AclRoleEntry>,
    /// Default permissions for users without explicit role (including anonymous)
    default_publish: Vec<String>,
    default_subscribe: Vec<String>,
    /// Reference to auth provider for username lookups
    auth_provider: Arc<AuthProvider>,
}

/// Internal role entry with compiled patterns
struct AclRoleEntry {
    /// Publish patterns
    publish: Vec<String>,
    /// Subscribe patterns
    subscribe: Vec<String>,
}

impl AclProvider {
    /// Create a new ACL provider from configuration
    pub fn new(config: &AclConfig, auth_provider: Arc<AuthProvider>) -> Self {
        let mut roles = HashMap::new();

        for role in &config.roles {
            roles.insert(
                role.name.clone(),
                AclRoleEntry {
                    publish: role.publish.clone(),
                    subscribe: role.subscribe.clone(),
                },
            );
        }

        Self {
            enabled: config.enabled,
            roles,
            default_publish: config.default.publish.clone(),
            default_subscribe: config.default.subscribe.clone(),
            auth_provider,
        }
    }

    /// Check if ACL is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Check if topic matches pattern with variable substitution
    fn matches_pattern(
        pattern: &str,
        topic: &str,
        client_id: &str,
        username: Option<&str>,
    ) -> bool {
        // Substitute variables
        let pattern = pattern
            .replace("%c", client_id)
            .replace("%u", username.unwrap_or(""));

        Self::mqtt_pattern_match(&pattern, topic)
    }

    /// MQTT pattern matching with wildcards
    fn mqtt_pattern_match(pattern: &str, topic: &str) -> bool {
        let pattern_parts: Vec<&str> = pattern.split('/').collect();
        let topic_parts: Vec<&str> = topic.split('/').collect();

        let mut p_idx = 0;
        let mut t_idx = 0;

        while p_idx < pattern_parts.len() && t_idx < topic_parts.len() {
            let p = pattern_parts[p_idx];
            let t = topic_parts[t_idx];

            if p == "#" {
                // Multi-level wildcard - matches rest of topic
                return true;
            } else if p == "+" {
                // Single-level wildcard - matches one level
                p_idx += 1;
                t_idx += 1;
            } else if p == t {
                // Exact match
                p_idx += 1;
                t_idx += 1;
            } else {
                // No match
                return false;
            }
        }

        // Both must be exhausted, or pattern ends with #
        p_idx == pattern_parts.len() && t_idx == topic_parts.len()
    }

    /// Check if any pattern in the list matches the topic
    fn check_patterns(
        patterns: &[String],
        topic: &str,
        client_id: &str,
        username: Option<&str>,
    ) -> bool {
        patterns
            .iter()
            .any(|p| Self::matches_pattern(p, topic, client_id, username))
    }

    /// Get role permissions for a username
    fn get_role_permissions(&self, username: Option<&str>) -> Option<&AclRoleEntry> {
        let username = username?;
        let role_name = self.auth_provider.get_user_role(username)?;
        self.roles.get(role_name)
    }
}

#[async_trait]
impl Hooks for AclProvider {
    async fn on_publish_check(
        &self,
        client_id: &str,
        username: Option<&str>,
        topic: &str,
        _qos: QoS,
        _retain: bool,
    ) -> HookResult<bool> {
        // If ACL is disabled, allow all
        if !self.enabled {
            return Ok(true);
        }

        // Try to get the actual username from auth provider
        let actual_username = self.auth_provider.get_client_username(client_id);
        let username_ref = actual_username.as_deref().or(username);

        // Check role-based permissions first
        if let Some(role) = self.get_role_permissions(username_ref) {
            if Self::check_patterns(&role.publish, topic, client_id, username_ref) {
                return Ok(true);
            }
        }

        // Check default permissions (applies to all users without a role, including anonymous)
        if Self::check_patterns(&self.default_publish, topic, client_id, username_ref) {
            return Ok(true);
        }

        // Deny by default
        Ok(false)
    }

    async fn on_subscribe_check(
        &self,
        client_id: &str,
        username: Option<&str>,
        filter: &str,
        _qos: QoS,
    ) -> HookResult<bool> {
        // If ACL is disabled, allow all
        if !self.enabled {
            return Ok(true);
        }

        // Try to get the actual username from auth provider
        let actual_username = self.auth_provider.get_client_username(client_id);
        let username_ref = actual_username.as_deref().or(username);

        // Check role-based permissions first
        if let Some(role) = self.get_role_permissions(username_ref) {
            if Self::check_patterns(&role.subscribe, filter, client_id, username_ref) {
                return Ok(true);
            }
        }

        // Check default permissions (applies to all users without a role, including anonymous)
        if Self::check_patterns(&self.default_subscribe, filter, client_id, username_ref) {
            return Ok(true);
        }

        // Deny by default
        Ok(false)
    }
}

#[cfg(test)]
mod pattern_tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        assert!(AclProvider::mqtt_pattern_match("foo/bar", "foo/bar"));
        assert!(!AclProvider::mqtt_pattern_match("foo/bar", "foo/baz"));
    }

    #[test]
    fn test_single_level_wildcard() {
        assert!(AclProvider::mqtt_pattern_match("foo/+/bar", "foo/xxx/bar"));
        assert!(AclProvider::mqtt_pattern_match("+/bar", "foo/bar"));
        assert!(!AclProvider::mqtt_pattern_match("foo/+", "foo/bar/baz"));
    }

    #[test]
    fn test_multi_level_wildcard() {
        assert!(AclProvider::mqtt_pattern_match("foo/#", "foo/bar"));
        assert!(AclProvider::mqtt_pattern_match("foo/#", "foo/bar/baz"));
        assert!(AclProvider::mqtt_pattern_match("#", "foo/bar/baz"));
    }

    #[test]
    fn test_variable_substitution() {
        assert!(AclProvider::matches_pattern(
            "sensors/%c/#",
            "sensors/client1/temp",
            "client1",
            None
        ));
        assert!(AclProvider::matches_pattern(
            "users/%u/data",
            "users/admin/data",
            "client1",
            Some("admin")
        ));
    }
}

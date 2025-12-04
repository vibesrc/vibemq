//! Hooks Module
//!
//! Provides extensibility points for authentication, authorization,
//! and custom event handling in VibeMQ.

use std::fmt;

use async_trait::async_trait;

use crate::protocol::QoS;

#[cfg(test)]
mod tests;

/// Hook error types
#[derive(Debug)]
pub enum HookError {
    /// Internal error
    Internal(String),
    /// Authentication failed
    AuthenticationFailed,
    /// Authorization denied
    AuthorizationDenied,
}

impl fmt::Display for HookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HookError::Internal(msg) => write!(f, "Internal error: {}", msg),
            HookError::AuthenticationFailed => write!(f, "Authentication failed"),
            HookError::AuthorizationDenied => write!(f, "Authorization denied"),
        }
    }
}

impl std::error::Error for HookError {}

/// Hook result type
pub type HookResult<T> = Result<T, HookError>;

/// Broker hooks trait
///
/// Implement this trait to customize authentication, authorization,
/// and event handling behavior. All methods have default implementations
/// that allow everything.
#[async_trait]
pub trait Hooks: Send + Sync {
    /// Called when a client attempts to authenticate
    ///
    /// # Arguments
    /// * `client_id` - The client identifier
    /// * `username` - Optional username from CONNECT packet
    /// * `password` - Optional password from CONNECT packet
    ///
    /// # Returns
    /// * `Ok(true)` - Authentication successful
    /// * `Ok(false)` - Authentication failed (will send CONNACK with error)
    /// * `Err(_)` - Internal error occurred
    async fn on_authenticate(
        &self,
        _client_id: &str,
        _username: Option<&str>,
        _password: Option<&[u8]>,
    ) -> HookResult<bool> {
        Ok(true) // Default: allow all
    }

    /// Called when a client attempts to publish a message
    ///
    /// # Arguments
    /// * `client_id` - The client identifier
    /// * `username` - The username used for authentication (if any)
    /// * `topic` - The topic being published to
    /// * `qos` - The QoS level of the publish
    /// * `retain` - Whether this is a retained message
    ///
    /// # Returns
    /// * `Ok(true)` - Publish allowed
    /// * `Ok(false)` - Publish denied
    /// * `Err(_)` - Internal error occurred
    async fn on_publish_check(
        &self,
        _client_id: &str,
        _username: Option<&str>,
        _topic: &str,
        _qos: QoS,
        _retain: bool,
    ) -> HookResult<bool> {
        Ok(true) // Default: allow all
    }

    /// Called when a client attempts to subscribe to a topic filter
    ///
    /// # Arguments
    /// * `client_id` - The client identifier
    /// * `username` - The username used for authentication (if any)
    /// * `filter` - The topic filter being subscribed to
    /// * `qos` - The requested QoS level
    ///
    /// # Returns
    /// * `Ok(true)` - Subscribe allowed
    /// * `Ok(false)` - Subscribe denied
    /// * `Err(_)` - Internal error occurred
    async fn on_subscribe_check(
        &self,
        _client_id: &str,
        _username: Option<&str>,
        _filter: &str,
        _qos: QoS,
    ) -> HookResult<bool> {
        Ok(true) // Default: allow all
    }

    /// Called after a client successfully connects
    ///
    /// This is called after authentication succeeds and CONNACK is sent.
    async fn on_client_connected(&self, _client_id: &str, _username: Option<&str>) {
        // Default: no-op
    }

    /// Called after a client disconnects
    ///
    /// # Arguments
    /// * `client_id` - The client identifier
    /// * `graceful` - Whether the disconnect was graceful (DISCONNECT packet received)
    async fn on_client_disconnected(&self, _client_id: &str, _graceful: bool) {
        // Default: no-op
    }

    /// Called after a message is successfully published
    ///
    /// This is called after the message has been routed to subscribers.
    async fn on_message_published(&self, _topic: &str, _payload: &[u8], _qos: QoS) {
        // Default: no-op
    }
}

/// Default hooks implementation that allows everything
pub struct DefaultHooks;

#[async_trait]
impl Hooks for DefaultHooks {
    // All methods use default implementations (allow all, no-op)
}

impl Default for DefaultHooks {
    fn default() -> Self {
        Self
    }
}

/// Implement Hooks for Arc<T> where T: Hooks
/// This allows Arc-wrapped hook providers to be used directly
#[async_trait]
impl<T: Hooks + ?Sized> Hooks for std::sync::Arc<T> {
    async fn on_authenticate(
        &self,
        client_id: &str,
        username: Option<&str>,
        password: Option<&[u8]>,
    ) -> HookResult<bool> {
        (**self)
            .on_authenticate(client_id, username, password)
            .await
    }

    async fn on_publish_check(
        &self,
        client_id: &str,
        username: Option<&str>,
        topic: &str,
        qos: QoS,
        retain: bool,
    ) -> HookResult<bool> {
        (**self)
            .on_publish_check(client_id, username, topic, qos, retain)
            .await
    }

    async fn on_subscribe_check(
        &self,
        client_id: &str,
        username: Option<&str>,
        filter: &str,
        qos: QoS,
    ) -> HookResult<bool> {
        (**self)
            .on_subscribe_check(client_id, username, filter, qos)
            .await
    }

    async fn on_client_connected(&self, client_id: &str, username: Option<&str>) {
        (**self).on_client_connected(client_id, username).await;
    }

    async fn on_client_disconnected(&self, client_id: &str, graceful: bool) {
        (**self).on_client_disconnected(client_id, graceful).await;
    }

    async fn on_message_published(&self, topic: &str, payload: &[u8], qos: QoS) {
        (**self).on_message_published(topic, payload, qos).await;
    }
}

/// Composite hooks that chains multiple hook implementations
///
/// For authentication: all hooks must return `Ok(true)` for success
/// For authorization: all hooks must return `Ok(true)` for permission
/// For events: all hooks are called in order
pub struct CompositeHooks {
    hooks: Vec<Box<dyn Hooks>>,
}

impl CompositeHooks {
    /// Create a new composite hooks instance
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    /// Add a hooks implementation
    pub fn add<H: Hooks + 'static>(&mut self, hooks: H) {
        self.hooks.push(Box::new(hooks));
    }

    /// Add a hooks implementation and return self for chaining
    pub fn with<H: Hooks + 'static>(mut self, hooks: H) -> Self {
        self.add(hooks);
        self
    }
}

impl Default for CompositeHooks {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Hooks for CompositeHooks {
    async fn on_authenticate(
        &self,
        client_id: &str,
        username: Option<&str>,
        password: Option<&[u8]>,
    ) -> HookResult<bool> {
        for hooks in &self.hooks {
            if !hooks.on_authenticate(client_id, username, password).await? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    async fn on_publish_check(
        &self,
        client_id: &str,
        username: Option<&str>,
        topic: &str,
        qos: QoS,
        retain: bool,
    ) -> HookResult<bool> {
        for hooks in &self.hooks {
            if !hooks
                .on_publish_check(client_id, username, topic, qos, retain)
                .await?
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    async fn on_subscribe_check(
        &self,
        client_id: &str,
        username: Option<&str>,
        filter: &str,
        qos: QoS,
    ) -> HookResult<bool> {
        for hooks in &self.hooks {
            if !hooks
                .on_subscribe_check(client_id, username, filter, qos)
                .await?
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    async fn on_client_connected(&self, client_id: &str, username: Option<&str>) {
        for hooks in &self.hooks {
            hooks.on_client_connected(client_id, username).await;
        }
    }

    async fn on_client_disconnected(&self, client_id: &str, graceful: bool) {
        for hooks in &self.hooks {
            hooks.on_client_disconnected(client_id, graceful).await;
        }
    }

    async fn on_message_published(&self, topic: &str, payload: &[u8], qos: QoS) {
        for hooks in &self.hooks {
            hooks.on_message_published(topic, payload, qos).await;
        }
    }
}

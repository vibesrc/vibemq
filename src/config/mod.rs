//! Configuration Module
//!
//! Provides TOML-based configuration for VibeMQ with support for:
//! - Server settings (bind address, workers)
//! - Connection limits
//! - Session parameters
//! - MQTT feature flags
//! - Authentication and ACL
//! - Bridge configuration
//! - Environment variable overrides (VIBEMQ_* prefix)

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use config::{Environment, File, FileFormat};
use regex::Regex;
use serde::Deserialize;

// Re-export bridge config types
pub use bridge::{
    BridgeConfig, BridgeProtocol, BridgeTlsConfig, ForwardDirection, ForwardRule, LoopPrevention,
};

// Re-export cluster config types
pub use cluster::ClusterConfig;

// Re-export metrics config types
pub use metrics::MetricsConfig;

mod bridge;
mod cluster;
mod metrics;

/// Substitute environment variables in a string.
/// Supports `${VAR}` and `${VAR:-default}` syntax.
fn substitute_env_vars(content: &str) -> String {
    let re = Regex::new(r"\$\{([^}:]+)(?::-([^}]*))?\}").unwrap();
    re.replace_all(content, |caps: &regex::Captures| {
        let var_name = &caps[1];
        let default = caps.get(2).map(|m| m.as_str()).unwrap_or("");
        std::env::var(var_name).unwrap_or_else(|_| default.to_string())
    })
    .to_string()
}

#[cfg(test)]
mod tests;

/// Configuration error types
#[derive(Debug)]
pub enum ConfigError {
    /// IO error reading config file
    Io(std::io::Error),
    /// TOML parsing error
    Parse(toml::de::Error),
    /// Config crate error
    Config(config::ConfigError),
    /// Validation error
    Validation(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(e) => write!(f, "IO error: {}", e),
            ConfigError::Parse(e) => write!(f, "Parse error: {}", e),
            ConfigError::Config(e) => write!(f, "Config error: {}", e),
            ConfigError::Validation(msg) => write!(f, "Validation error: {}", msg),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<std::io::Error> for ConfigError {
    fn from(e: std::io::Error) -> Self {
        ConfigError::Io(e)
    }
}

impl From<toml::de::Error> for ConfigError {
    fn from(e: toml::de::Error) -> Self {
        ConfigError::Parse(e)
    }
}

impl From<config::ConfigError> for ConfigError {
    fn from(e: config::ConfigError) -> Self {
        ConfigError::Config(e)
    }
}

/// Root configuration structure
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct Config {
    /// Logging configuration
    pub log: LogConfig,
    /// Server configuration
    pub server: ServerConfig,
    /// Connection limits
    pub limits: LimitsConfig,
    /// Session configuration
    pub session: SessionConfig,
    /// MQTT feature configuration
    pub mqtt: MqttConfig,
    /// Authentication configuration
    pub auth: AuthConfig,
    /// ACL configuration
    pub acl: AclConfig,
    /// Bridge configurations
    #[serde(default)]
    pub bridge: Vec<BridgeConfig>,
    /// Cluster configuration (only first entry is used if multiple)
    #[serde(default)]
    pub cluster: Vec<ClusterConfig>,
    /// Metrics configuration
    #[serde(default)]
    pub metrics: MetricsConfig,
}

/// Logging configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LogConfig {
    /// Log level: error, warn, info, debug, trace
    #[serde(default = "default_log_level")]
    pub level: String,
}

fn default_log_level() -> String {
    "info".to_string()
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
        }
    }
}

/// Server configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// TCP bind address
    #[serde(default = "default_bind")]
    pub bind: SocketAddr,
    /// TLS bind address (optional, enables MQTT over TLS)
    pub tls_bind: Option<SocketAddr>,
    /// WebSocket bind address (optional)
    pub ws_bind: Option<SocketAddr>,
    /// WebSocket path (default: "/mqtt")
    #[serde(default = "default_ws_path")]
    pub ws_path: String,
    /// Number of worker threads (0 = auto)
    #[serde(default)]
    pub workers: usize,
    /// TLS configuration (required when tls_bind is set)
    #[serde(default)]
    pub tls: Option<ServerTlsConfig>,
}

/// TLS configuration for the server
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ServerTlsConfig {
    /// Path to certificate file (PEM format)
    pub cert: String,
    /// Path to private key file (PEM format)
    pub key: String,
    /// Path to CA certificate file for client authentication (PEM format, optional)
    pub ca_cert: Option<String>,
    /// Require client certificate authentication
    #[serde(default)]
    pub require_client_cert: bool,
}

fn default_ws_path() -> String {
    "/mqtt".to_string()
}

fn default_bind() -> SocketAddr {
    "0.0.0.0:1883".parse().unwrap()
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            tls_bind: None,
            ws_bind: None,
            ws_path: default_ws_path(),
            workers: 0,
            tls: None,
        }
    }
}

/// Connection limits configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LimitsConfig {
    /// Maximum number of connections
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,
    /// Maximum packet size in bytes
    #[serde(default = "default_max_packet_size")]
    pub max_packet_size: usize,
    /// Maximum in-flight messages per client (QoS 1/2)
    #[serde(default = "default_max_inflight")]
    pub max_inflight: u16,
    /// Maximum queued messages per offline client
    #[serde(default = "default_max_queued_messages")]
    pub max_queued_messages: usize,
    /// Maximum pending PUBREL for QoS 2
    #[serde(default = "default_max_awaiting_rel")]
    pub max_awaiting_rel: usize,
    /// Seconds before retrying unacked messages
    #[serde(default = "default_retry_interval")]
    pub retry_interval: u64,
    /// Per-connection outbound message channel capacity.
    /// This buffer holds messages waiting to be written to the client socket.
    /// Higher values handle burst traffic better but use more memory per connection.
    /// Set to 0 for unbounded (not recommended for production).
    #[serde(default = "default_outbound_channel_capacity")]
    pub outbound_channel_capacity: usize,
}

fn default_max_connections() -> usize {
    100_000
}
fn default_max_packet_size() -> usize {
    1024 * 1024
}
fn default_max_inflight() -> u16 {
    32
}
fn default_max_queued_messages() -> usize {
    1000
}
fn default_max_awaiting_rel() -> usize {
    100
}
fn default_retry_interval() -> u64 {
    30
}
fn default_outbound_channel_capacity() -> usize {
    1024
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            max_connections: default_max_connections(),
            max_packet_size: default_max_packet_size(),
            max_inflight: default_max_inflight(),
            max_queued_messages: default_max_queued_messages(),
            max_awaiting_rel: default_max_awaiting_rel(),
            retry_interval: default_retry_interval(),
            outbound_channel_capacity: default_outbound_channel_capacity(),
        }
    }
}

impl LimitsConfig {
    /// Get retry interval as Duration
    pub fn retry_interval_duration(&self) -> Duration {
        Duration::from_secs(self.retry_interval)
    }
}

/// Session configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SessionConfig {
    /// Default keep alive in seconds
    #[serde(default = "default_keep_alive")]
    pub default_keep_alive: u16,
    /// Maximum keep alive in seconds
    #[serde(default = "default_max_keep_alive")]
    pub max_keep_alive: u16,
    /// Session expiry check interval in seconds
    #[serde(default = "default_expiry_check_interval")]
    pub expiry_check_interval: u64,
    /// Maximum topic aliases
    #[serde(default = "default_max_topic_aliases")]
    pub max_topic_aliases: u16,
}

fn default_keep_alive() -> u16 {
    60
}
fn default_max_keep_alive() -> u16 {
    65535
}
fn default_expiry_check_interval() -> u64 {
    60
}
fn default_max_topic_aliases() -> u16 {
    65535
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            default_keep_alive: default_keep_alive(),
            max_keep_alive: default_max_keep_alive(),
            expiry_check_interval: default_expiry_check_interval(),
            max_topic_aliases: default_max_topic_aliases(),
        }
    }
}

impl SessionConfig {
    /// Get expiry check interval as Duration
    pub fn expiry_check_interval_duration(&self) -> Duration {
        Duration::from_secs(self.expiry_check_interval)
    }
}

/// MQTT feature configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MqttConfig {
    /// Maximum QoS level (0, 1, or 2)
    #[serde(default = "default_max_qos")]
    pub max_qos: u8,
    /// Whether retained messages are available
    #[serde(default = "default_true")]
    pub retain_available: bool,
    /// Whether wildcard subscriptions are available
    #[serde(default = "default_true")]
    pub wildcard_subscriptions: bool,
    /// Whether subscription identifiers are available
    #[serde(default = "default_true")]
    pub subscription_identifiers: bool,
    /// Whether shared subscriptions are available
    #[serde(default = "default_true")]
    pub shared_subscriptions: bool,
    /// Whether $SYS topics are published
    #[serde(default = "default_true")]
    pub sys_topics: bool,
    /// $SYS topic publish interval in seconds
    #[serde(default = "default_sys_interval")]
    pub sys_interval: u64,
}

fn default_max_qos() -> u8 {
    2
}
fn default_true() -> bool {
    true
}
fn default_sys_interval() -> u64 {
    10
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self {
            max_qos: default_max_qos(),
            retain_available: true,
            wildcard_subscriptions: true,
            subscription_identifiers: true,
            shared_subscriptions: true,
            sys_topics: true,
            sys_interval: default_sys_interval(),
        }
    }
}

/// Authentication configuration
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct AuthConfig {
    /// Whether authentication is enabled
    pub enabled: bool,
    /// Allow anonymous connections when auth is enabled
    #[serde(default = "default_true")]
    pub allow_anonymous: bool,
    /// Static user list
    #[serde(default)]
    pub users: Vec<UserConfig>,
}

/// User configuration
#[derive(Debug, Clone, Deserialize)]
pub struct UserConfig {
    /// Username
    pub username: String,
    /// Password (plaintext) - use password_hash for production
    #[serde(default)]
    pub password: Option<String>,
    /// Password hash (argon2 PHC format: $argon2id$v=19$...)
    #[serde(default)]
    pub password_hash: Option<String>,
    /// Role name for ACL permissions
    #[serde(default)]
    pub role: Option<String>,
}

/// ACL configuration
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct AclConfig {
    /// Whether ACL is enabled
    pub enabled: bool,
    /// ACL roles
    #[serde(default)]
    pub roles: Vec<AclRole>,
    /// Default permissions for users without explicit role (including anonymous)
    #[serde(default)]
    pub default: AclPermissions,
}

/// ACL role
#[derive(Debug, Clone, Deserialize)]
pub struct AclRole {
    /// Role name
    pub name: String,
    /// Topic patterns this role can publish to
    #[serde(default)]
    pub publish: Vec<String>,
    /// Topic patterns this role can subscribe to
    #[serde(default)]
    pub subscribe: Vec<String>,
}

/// ACL permissions
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct AclPermissions {
    /// Topic patterns that can be published to
    pub publish: Vec<String>,
    /// Topic patterns that can be subscribed to
    pub subscribe: Vec<String>,
}

impl Config {
    /// Load configuration from a TOML file with environment variable overrides.
    ///
    /// Supports two forms of environment variable usage:
    /// 1. In-file substitution: `${VAR}` or `${VAR:-default}` syntax in the TOML file
    /// 2. Override via env vars: `VIBEMQ__` prefix with double underscores for nesting:
    ///    - `VIBEMQ__SERVER__BIND=0.0.0.0:1884` overrides `server.bind`
    ///    - `VIBEMQ__LIMITS__MAX_CONNECTIONS=50000` overrides `limits.max_connections`
    ///    - `VIBEMQ__AUTH__ENABLED=true` overrides `auth.enabled`
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let mut builder = config::Config::builder()
            // Start with defaults
            .set_default("log.level", "info")?
            .set_default("server.bind", "0.0.0.0:1883")?
            .set_default("server.ws_path", "/mqtt")?
            .set_default("server.workers", 0)?
            .set_default("limits.max_connections", 100_000)?
            .set_default("limits.max_packet_size", 1024 * 1024)?
            .set_default("limits.max_inflight", 32)?
            .set_default("limits.max_queued_messages", 1000)?
            .set_default("limits.max_awaiting_rel", 100)?
            .set_default("limits.retry_interval", 30)?
            .set_default("limits.outbound_channel_capacity", 1024)?
            .set_default("session.default_keep_alive", 60)?
            .set_default("session.max_keep_alive", 65535)?
            .set_default("session.expiry_check_interval", 60)?
            .set_default("session.max_topic_aliases", 65535)?
            .set_default("mqtt.max_qos", 2)?
            .set_default("mqtt.retain_available", true)?
            .set_default("mqtt.wildcard_subscriptions", true)?
            .set_default("mqtt.subscription_identifiers", true)?
            .set_default("mqtt.shared_subscriptions", true)?
            .set_default("auth.enabled", false)?
            .set_default("auth.allow_anonymous", true)?
            .set_default("acl.enabled", false)?;

        // Load from file with env var substitution
        let path = path.as_ref();
        match std::fs::read_to_string(path) {
            Ok(content) => {
                let substituted = substitute_env_vars(&content);
                builder = builder.add_source(File::from_str(&substituted, FileFormat::Toml));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // File doesn't exist, use defaults
            }
            Err(e) => return Err(ConfigError::Io(e)),
        }

        // Override with environment variables (VIBEMQ__SERVER__BIND, etc.)
        // Double underscore separates nested keys, single underscore preserved in field names
        let cfg = builder
            .add_source(
                Environment::with_prefix("VIBEMQ")
                    .separator("__")
                    .try_parsing(true),
            )
            .build()?;

        let config: Config = cfg.try_deserialize()?;
        config.validate()?;
        Ok(config)
    }

    /// Load configuration with environment variable overrides only (no file).
    ///
    /// Useful for containerized deployments where all config comes from env vars.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::load(Path::new(""))
    }

    /// Parse configuration from a string (for testing, no env var support)
    pub fn parse(content: &str) -> Result<Self, ConfigError> {
        let config: Config = toml::from_str(content)?;
        config.validate()?;
        Ok(config)
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Validate max_qos
        if self.mqtt.max_qos > 2 {
            return Err(ConfigError::Validation(
                "max_qos must be 0, 1, or 2".to_string(),
            ));
        }

        // Note: 0 means unbounded for all limits

        // Validate user password configuration
        if self.auth.enabled {
            for user in &self.auth.users {
                match (&user.password, &user.password_hash) {
                    (None, None) => {
                        return Err(ConfigError::Validation(format!(
                            "User '{}' must have either 'password' or 'password_hash'",
                            user.username
                        )));
                    }
                    (Some(_), Some(_)) => {
                        return Err(ConfigError::Validation(format!(
                            "User '{}' cannot have both 'password' and 'password_hash'",
                            user.username
                        )));
                    }
                    (Some(pwd), None) if pwd.is_empty() => {
                        return Err(ConfigError::Validation(format!(
                            "User '{}' has empty password",
                            user.username
                        )));
                    }
                    (None, Some(hash)) if !hash.starts_with("$argon2") => {
                        return Err(ConfigError::Validation(format!(
                            "User '{}' has invalid password_hash format (must be argon2 PHC format)",
                            user.username
                        )));
                    }
                    _ => {}
                }
            }
        }

        // Validate ACL role references
        if self.auth.enabled && self.acl.enabled {
            let role_names: std::collections::HashSet<_> =
                self.acl.roles.iter().map(|r| &r.name).collect();

            for user in &self.auth.users {
                if let Some(ref role) = user.role {
                    if !role_names.contains(role) {
                        return Err(ConfigError::Validation(format!(
                            "User '{}' references unknown role '{}'",
                            user.username, role
                        )));
                    }
                }
            }
        }

        // Validate TLS configuration
        if self.server.tls_bind.is_some() {
            match &self.server.tls {
                Some(tls) => {
                    if tls.cert.is_empty() {
                        return Err(ConfigError::Validation(
                            "tls.cert is required when tls_bind is set".to_string(),
                        ));
                    }
                    if tls.key.is_empty() {
                        return Err(ConfigError::Validation(
                            "tls.key is required when tls_bind is set".to_string(),
                        ));
                    }
                }
                None => {
                    return Err(ConfigError::Validation(
                        "tls configuration is required when tls_bind is set".to_string(),
                    ));
                }
            }
        }

        Ok(())
    }

    /// Build a role lookup map for efficient ACL checks
    pub fn build_role_map(&self) -> HashMap<String, &AclRole> {
        self.acl
            .roles
            .iter()
            .map(|role| (role.name.clone(), role))
            .collect()
    }

    /// Build a user lookup map for efficient auth checks
    pub fn build_user_map(&self) -> HashMap<String, &UserConfig> {
        self.auth
            .users
            .iter()
            .map(|user| (user.username.clone(), user))
            .collect()
    }
}

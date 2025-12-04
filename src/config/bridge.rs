//! Bridge Configuration
//!
//! Configuration structures for MQTT bridge connections.

use std::time::Duration;

use serde::Deserialize;

/// Bridge connection protocol
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BridgeProtocol {
    /// Plain MQTT over TCP
    #[default]
    Mqtt,
    /// MQTT over TLS
    Mqtts,
    /// MQTT over WebSocket
    Ws,
    /// MQTT over WebSocket with TLS
    Wss,
}

impl std::fmt::Display for BridgeProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeProtocol::Mqtt => write!(f, "mqtt"),
            BridgeProtocol::Mqtts => write!(f, "mqtts"),
            BridgeProtocol::Ws => write!(f, "ws"),
            BridgeProtocol::Wss => write!(f, "wss"),
        }
    }
}

impl BridgeProtocol {
    /// Get default port for this protocol
    pub fn default_port(&self) -> u16 {
        match self {
            BridgeProtocol::Mqtt => 1883,
            BridgeProtocol::Mqtts => 8883,
            BridgeProtocol::Ws => 80,
            BridgeProtocol::Wss => 443,
        }
    }

    /// Check if this protocol uses TLS
    pub fn uses_tls(&self) -> bool {
        matches!(self, BridgeProtocol::Mqtts | BridgeProtocol::Wss)
    }

    /// Check if this protocol uses WebSocket
    pub fn uses_websocket(&self) -> bool {
        matches!(self, BridgeProtocol::Ws | BridgeProtocol::Wss)
    }
}

/// Direction of message forwarding
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ForwardDirection {
    /// Forward messages from local broker to remote (publish locally → publish remotely)
    #[default]
    Out,
    /// Forward messages from remote broker to local (subscribe remotely → publish locally)
    In,
    /// Bidirectional forwarding
    Both,
}

/// Topic forwarding rule
#[derive(Debug, Clone, Deserialize)]
pub struct ForwardRule {
    /// Topic pattern on local broker
    #[serde(alias = "local")]
    pub local_topic: String,

    /// Topic pattern on remote broker (supports `{local}` placeholder)
    #[serde(alias = "remote")]
    pub remote_topic: String,

    /// Direction of forwarding
    #[serde(default)]
    pub direction: ForwardDirection,

    /// QoS level for forwarded messages (caps the original QoS)
    #[serde(default = "default_qos")]
    pub qos: u8,

    /// Whether to forward retained messages
    #[serde(default = "default_true")]
    pub retain: bool,
}

/// Loop prevention strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopPrevention {
    /// Use MQTT v5.0 no_local subscription flag (default, most efficient)
    #[default]
    NoLocal,
    /// Use user property to tag messages with origin broker
    UserProperty,
    /// Use both no_local and user property (most robust)
    Both,
    /// Disable loop prevention (use with caution!)
    None,
}

fn default_qos() -> u8 {
    1
}

fn default_true() -> bool {
    true
}

impl ForwardRule {
    /// Check if this rule applies to outbound messages (local → remote)
    pub fn is_outbound(&self) -> bool {
        matches!(
            self.direction,
            ForwardDirection::Out | ForwardDirection::Both
        )
    }

    /// Check if this rule applies to inbound messages (remote → local)
    pub fn is_inbound(&self) -> bool {
        matches!(
            self.direction,
            ForwardDirection::In | ForwardDirection::Both
        )
    }
}

/// Configuration for a single bridge connection
#[derive(Debug, Clone, Deserialize)]
pub struct BridgeConfig {
    /// Unique name for this bridge
    pub name: String,

    /// Remote broker address (host:port or just host)
    pub address: String,

    /// Connection protocol
    #[serde(default)]
    pub protocol: BridgeProtocol,

    /// Client ID to use when connecting to remote broker
    #[serde(default = "default_client_id")]
    pub client_id: String,

    /// Username for authentication
    pub username: Option<String>,

    /// Password for authentication
    pub password: Option<String>,

    /// Keep-alive interval in seconds
    #[serde(default = "default_keepalive")]
    pub keepalive: u16,

    /// Use clean start (no session persistence)
    #[serde(default = "default_true")]
    pub clean_start: bool,

    /// Reconnect interval in seconds
    #[serde(default = "default_reconnect_interval")]
    pub reconnect_interval: u64,

    /// Maximum reconnect interval in seconds (for exponential backoff)
    #[serde(default = "default_max_reconnect_interval")]
    pub max_reconnect_interval: u64,

    /// Connection timeout in seconds
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout: u64,

    /// Topic forwarding rules
    #[serde(default, alias = "forward")]
    pub forwards: Vec<ForwardRule>,

    /// TLS configuration (when using mqtts or wss)
    #[serde(default)]
    pub tls: Option<BridgeTlsConfig>,

    /// WebSocket path (when using ws or wss)
    #[serde(default = "default_ws_path")]
    pub ws_path: String,

    /// Whether this bridge is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Loop prevention strategy
    #[serde(default)]
    pub loop_prevention: LoopPrevention,

    /// Origin identifier for user property loop prevention
    /// Defaults to the bridge name if not specified
    #[serde(default)]
    pub origin_id: Option<String>,
}

fn default_client_id() -> String {
    format!("vibemq-bridge-{}", std::process::id())
}

fn default_keepalive() -> u16 {
    60
}

fn default_reconnect_interval() -> u64 {
    5
}

fn default_max_reconnect_interval() -> u64 {
    60
}

fn default_connect_timeout() -> u64 {
    30
}

fn default_ws_path() -> String {
    "/mqtt".to_string()
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            address: "localhost:1883".to_string(),
            protocol: BridgeProtocol::default(),
            client_id: default_client_id(),
            username: None,
            password: None,
            keepalive: default_keepalive(),
            clean_start: true,
            reconnect_interval: default_reconnect_interval(),
            max_reconnect_interval: default_max_reconnect_interval(),
            connect_timeout: default_connect_timeout(),
            forwards: Vec::new(),
            tls: None,
            ws_path: default_ws_path(),
            enabled: true,
            loop_prevention: LoopPrevention::default(),
            origin_id: None,
        }
    }
}

impl BridgeConfig {
    /// Get the reconnect interval as Duration
    pub fn reconnect_interval_duration(&self) -> Duration {
        Duration::from_secs(self.reconnect_interval)
    }

    /// Get the max reconnect interval as Duration
    pub fn max_reconnect_interval_duration(&self) -> Duration {
        Duration::from_secs(self.max_reconnect_interval)
    }

    /// Get the connect timeout as Duration
    pub fn connect_timeout_duration(&self) -> Duration {
        Duration::from_secs(self.connect_timeout)
    }

    /// Parse address into host and port
    pub fn parse_address(&self) -> (String, u16) {
        if let Some((host, port_str)) = self.address.rsplit_once(':') {
            if let Ok(port) = port_str.parse::<u16>() {
                return (host.to_string(), port);
            }
        }
        (self.address.clone(), self.protocol.default_port())
    }

    /// Get outbound forwarding rules (local → remote)
    pub fn outbound_rules(&self) -> impl Iterator<Item = &ForwardRule> {
        self.forwards.iter().filter(|r| r.is_outbound())
    }

    /// Get inbound forwarding rules (remote → local)
    pub fn inbound_rules(&self) -> impl Iterator<Item = &ForwardRule> {
        self.forwards.iter().filter(|r| r.is_inbound())
    }

    /// Get the origin identifier for loop prevention
    pub fn get_origin_id(&self) -> &str {
        self.origin_id.as_deref().unwrap_or(&self.name)
    }

    /// Check if no_local should be used for subscriptions
    pub fn use_no_local(&self) -> bool {
        matches!(
            self.loop_prevention,
            LoopPrevention::NoLocal | LoopPrevention::Both
        )
    }

    /// Check if user property tagging should be used
    pub fn use_origin_property(&self) -> bool {
        matches!(
            self.loop_prevention,
            LoopPrevention::UserProperty | LoopPrevention::Both
        )
    }
}

/// TLS configuration for bridge connections
#[derive(Debug, Clone, Deserialize, Default)]
pub struct BridgeTlsConfig {
    /// Path to CA certificate file (PEM format)
    pub ca_cert: Option<String>,

    /// Path to client certificate file (PEM format)
    pub client_cert: Option<String>,

    /// Path to client private key file (PEM format)
    pub client_key: Option<String>,

    /// Skip server certificate verification (insecure, for testing only)
    #[serde(default)]
    pub insecure: bool,

    /// Server name for SNI (defaults to address hostname)
    pub server_name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_address_with_port() {
        let config = BridgeConfig {
            address: "broker.example.com:8883".to_string(),
            ..Default::default()
        };
        let (host, port) = config.parse_address();
        assert_eq!(host, "broker.example.com");
        assert_eq!(port, 8883);
    }

    #[test]
    fn test_parse_address_without_port() {
        let config = BridgeConfig {
            address: "broker.example.com".to_string(),
            protocol: BridgeProtocol::Mqtts,
            ..Default::default()
        };
        let (host, port) = config.parse_address();
        assert_eq!(host, "broker.example.com");
        assert_eq!(port, 8883); // Default for mqtts
    }

    #[test]
    fn test_forward_direction() {
        let out_rule = ForwardRule {
            local_topic: "local/#".to_string(),
            remote_topic: "remote/#".to_string(),
            direction: ForwardDirection::Out,
            qos: 1,
            retain: true,
        };
        assert!(out_rule.is_outbound());
        assert!(!out_rule.is_inbound());

        let both_rule = ForwardRule {
            direction: ForwardDirection::Both,
            ..out_rule.clone()
        };
        assert!(both_rule.is_outbound());
        assert!(both_rule.is_inbound());
    }

    #[test]
    fn test_protocol_defaults() {
        assert_eq!(BridgeProtocol::Mqtt.default_port(), 1883);
        assert_eq!(BridgeProtocol::Mqtts.default_port(), 8883);
        assert!(BridgeProtocol::Mqtts.uses_tls());
        assert!(!BridgeProtocol::Mqtt.uses_tls());
        assert!(BridgeProtocol::Ws.uses_websocket());
        assert!(!BridgeProtocol::Mqtt.uses_websocket());
    }
}

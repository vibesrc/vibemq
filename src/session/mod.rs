//! MQTT Session Management
//!
//! Handles session state, message queues, and packet identifier tracking
//! for both persistent (clean_start=false) and non-persistent sessions.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use dashmap::DashMap;
use parking_lot::RwLock;

use crate::protocol::{Properties, ProtocolVersion, Publish, QoS, SubscriptionOptions};

/// Session state
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    /// Session is connected
    Connected,
    /// Session is disconnected but persisted
    Disconnected,
    /// Session has expired
    Expired,
}

/// Inflight message state for QoS 1/2
#[derive(Debug, Clone)]
pub struct InflightMessage {
    /// Packet identifier
    pub packet_id: u16,
    /// The publish packet
    pub publish: Publish,
    /// QoS 2 state
    pub qos2_state: Option<Qos2State>,
    /// Timestamp when the message was sent
    pub sent_at: Instant,
    /// Number of retransmission attempts
    pub retry_count: u32,
}

/// QoS 2 message state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Qos2State {
    /// PUBLISH sent, waiting for PUBREC
    WaitingPubRec,
    /// PUBREC received, PUBREL sent, waiting for PUBCOMP
    WaitingPubComp,
}

/// Subscription entry in session
#[derive(Debug, Clone)]
pub struct SessionSubscription {
    /// Topic filter
    pub filter: String,
    /// Subscription options
    pub options: SubscriptionOptions,
    /// Subscription identifier (v5.0)
    pub subscription_id: Option<u32>,
}

/// Client session
pub struct Session {
    /// Client identifier
    pub client_id: Arc<str>,
    /// Protocol version
    pub protocol_version: ProtocolVersion,
    /// Session state
    pub state: SessionState,
    /// Clean start flag
    pub clean_start: bool,
    /// Session expiry interval in seconds (0 = delete on disconnect)
    pub session_expiry_interval: u32,
    /// Keep alive interval in seconds
    pub keep_alive: u16,
    /// Last activity timestamp
    pub last_activity: Instant,
    /// Subscriptions
    pub subscriptions: HashMap<String, SessionSubscription>,
    /// Inflight outgoing messages (QoS 1/2)
    pub inflight_outgoing: HashMap<u16, InflightMessage>,
    /// Inflight incoming messages (QoS 2)
    pub inflight_incoming: HashMap<u16, Qos2State>,
    /// Next packet identifier
    next_packet_id: u16,
    /// Pending messages (queued while disconnected)
    pub pending_messages: VecDeque<Publish>,
    /// Maximum pending messages
    pub max_pending_messages: usize,
    /// Receive maximum (flow control)
    pub receive_maximum: u16,
    /// Current send quota
    pub send_quota: u16,
    /// Maximum packet size
    pub max_packet_size: u32,
    /// Topic aliases (client -> server)
    pub client_topic_aliases: HashMap<u16, String>,
    /// Topic aliases (server -> client)
    pub server_topic_aliases: HashMap<String, u16>,
    /// Next server topic alias
    next_server_alias: u16,
    /// Maximum topic alias
    pub topic_alias_maximum: u16,
    /// Will message
    pub will: Option<WillMessage>,
    /// Will delay interval
    pub will_delay_interval: u32,
    /// Disconnect timestamp
    pub disconnected_at: Option<Instant>,
}

/// Will message
#[derive(Debug, Clone)]
pub struct WillMessage {
    pub topic: String,
    pub payload: Bytes,
    pub qos: QoS,
    pub retain: bool,
    pub properties: Properties,
}

impl Session {
    pub fn new(client_id: Arc<str>, protocol_version: ProtocolVersion) -> Self {
        Self {
            client_id,
            protocol_version,
            state: SessionState::Connected,
            clean_start: true,
            session_expiry_interval: 0,
            keep_alive: 60,
            last_activity: Instant::now(),
            subscriptions: HashMap::new(),
            inflight_outgoing: HashMap::new(),
            inflight_incoming: HashMap::new(),
            next_packet_id: 1,
            pending_messages: VecDeque::new(),
            max_pending_messages: 1000,
            receive_maximum: 65535,
            send_quota: 65535,
            max_packet_size: 268_435_455,
            client_topic_aliases: HashMap::new(),
            server_topic_aliases: HashMap::new(),
            next_server_alias: 1,
            topic_alias_maximum: 0,
            will: None,
            will_delay_interval: 0,
            disconnected_at: None,
        }
    }

    /// Get next available packet identifier
    pub fn next_packet_id(&mut self) -> u16 {
        loop {
            let id = self.next_packet_id;
            self.next_packet_id = self.next_packet_id.wrapping_add(1);
            if self.next_packet_id == 0 {
                self.next_packet_id = 1;
            }

            // Make sure this ID is not in use
            if !self.inflight_outgoing.contains_key(&id)
                && !self.inflight_incoming.contains_key(&id)
            {
                return id;
            }
        }
    }

    /// Update last activity timestamp
    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    /// Check if session has expired
    pub fn is_expired(&self) -> bool {
        if self.state != SessionState::Disconnected {
            return false;
        }

        if self.session_expiry_interval == 0 {
            return true;
        }

        if self.session_expiry_interval == 0xFFFFFFFF {
            return false; // Never expires
        }

        if let Some(disconnected_at) = self.disconnected_at {
            let elapsed = disconnected_at.elapsed();
            return elapsed.as_secs() >= self.session_expiry_interval as u64;
        }

        false
    }

    /// Check if keep alive has timed out
    pub fn is_keep_alive_expired(&self) -> bool {
        if self.keep_alive == 0 {
            return false;
        }

        // Per spec, server can disconnect after 1.5 * keep_alive
        let timeout = Duration::from_secs((self.keep_alive as u64 * 3) / 2);
        self.last_activity.elapsed() > timeout
    }

    /// Queue a message for later delivery
    pub fn queue_message(&mut self, publish: Publish) -> bool {
        if self.pending_messages.len() >= self.max_pending_messages {
            // Drop oldest message
            self.pending_messages.pop_front();
        }
        self.pending_messages.push_back(publish);
        true
    }

    /// Get and remove pending messages
    pub fn drain_pending_messages(&mut self) -> VecDeque<Publish> {
        std::mem::take(&mut self.pending_messages)
    }

    /// Add a subscription
    pub fn add_subscription(
        &mut self,
        filter: String,
        options: SubscriptionOptions,
        subscription_id: Option<u32>,
    ) {
        self.subscriptions.insert(
            filter.clone(),
            SessionSubscription {
                filter,
                options,
                subscription_id,
            },
        );
    }

    /// Remove a subscription
    pub fn remove_subscription(&mut self, filter: &str) -> bool {
        self.subscriptions.remove(filter).is_some()
    }

    /// Get a topic alias for server->client
    pub fn get_or_create_topic_alias(&mut self, topic: &str) -> Option<u16> {
        if self.topic_alias_maximum == 0 {
            return None;
        }

        if let Some(&alias) = self.server_topic_aliases.get(topic) {
            return Some(alias);
        }

        if self.next_server_alias <= self.topic_alias_maximum {
            let alias = self.next_server_alias;
            self.next_server_alias += 1;
            self.server_topic_aliases.insert(topic.to_string(), alias);
            Some(alias)
        } else {
            None
        }
    }

    /// Resolve a client topic alias
    pub fn resolve_topic_alias(&self, alias: u16) -> Option<&String> {
        self.client_topic_aliases.get(&alias)
    }

    /// Register a client topic alias
    pub fn register_topic_alias(&mut self, alias: u16, topic: String) {
        self.client_topic_aliases.insert(alias, topic);
    }

    /// Decrement send quota (for flow control)
    pub fn decrement_send_quota(&mut self) -> bool {
        if self.send_quota > 0 {
            self.send_quota -= 1;
            true
        } else {
            false
        }
    }

    /// Increment send quota (on ack received)
    pub fn increment_send_quota(&mut self) {
        if self.send_quota < self.receive_maximum {
            self.send_quota += 1;
        }
    }
}

/// Thread-safe session store
pub struct SessionStore {
    sessions: DashMap<Arc<str>, Arc<RwLock<Session>>>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
        }
    }

    /// Get or create a session
    pub fn get_or_create(
        &self,
        client_id: &str,
        protocol_version: ProtocolVersion,
        clean_start: bool,
    ) -> (Arc<RwLock<Session>>, bool) {
        let client_id: Arc<str> = client_id.into();

        if clean_start {
            // Create new session
            let session = Arc::new(RwLock::new(Session::new(
                client_id.clone(),
                protocol_version,
            )));
            self.sessions.insert(client_id, session.clone());
            (session, false)
        } else {
            // Try to resume existing session
            if let Some(session) = self.sessions.get(&client_id) {
                let mut s = session.write();
                if !s.is_expired() {
                    s.state = SessionState::Connected;
                    s.protocol_version = protocol_version;
                    s.disconnected_at = None;
                    drop(s);
                    return (session.clone(), true);
                }
            }

            // Create new session
            let session = Arc::new(RwLock::new(Session::new(
                client_id.clone(),
                protocol_version,
            )));
            self.sessions.insert(client_id, session.clone());
            (session, false)
        }
    }

    /// Get a session by client ID
    pub fn get(&self, client_id: &str) -> Option<Arc<RwLock<Session>>> {
        self.sessions.get(client_id).map(|r| r.clone())
    }

    /// Remove a session
    pub fn remove(&self, client_id: &str) {
        self.sessions.remove(client_id);
    }

    /// Mark session as disconnected
    pub fn disconnect(&self, client_id: &str) {
        let should_remove = if let Some(session) = self.sessions.get(client_id) {
            let mut s = session.write();
            s.state = SessionState::Disconnected;
            s.disconnected_at = Some(Instant::now());
            s.session_expiry_interval == 0
        } else {
            false
        };

        // Remove after releasing the DashMap read lock to avoid deadlock
        if should_remove {
            self.sessions.remove(client_id);
        }
    }

    /// Clean up expired sessions
    pub fn cleanup_expired(&self) {
        self.sessions.retain(|_, session| {
            let s = session.read();
            !s.is_expired()
        });
    }

    /// Get session count
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new()
    }
}

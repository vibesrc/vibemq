//! MQTT Connection Handler
//!
//! Handles individual client connections, packet processing,
//! and protocol state machine.
//!
//! Performance optimizations:
//! - Uses AHashMap for faster deduplication during message routing
//! - Uses SmallVec for subscription IDs (typically few per message)
//! - Pre-allocates collections with reasonable capacity

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ahash::AHashMap;
use bytes::BytesMut;
use dashmap::DashMap;
use parking_lot::RwLock;
use smallvec::SmallVec;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{broadcast, mpsc};
use tokio::time::timeout;
use tracing::{debug, error, trace, warn};

use crate::broker::{BrokerConfig, BrokerEvent, RetainedMessage};
use crate::codec::{Decoder, Encoder};
use crate::hooks::Hooks;
use crate::metrics::Metrics;
use crate::protocol::{
    ConnAck, Connect, Disconnect, Packet, Properties, ProtocolVersion, PubAck, PubComp, PubRec,
    PubRel, Publish, QoS, ReasonCode, RetainHandling, SubAck, Subscribe, UnsubAck, Unsubscribe,
};
use crate::session::{InflightMessage, Qos2State, QueueResult, Session, SessionStore, WillMessage};
use crate::topic::{validate_topic_filter, validate_topic_name, Subscription, SubscriptionStore};

/// Connection error types
#[derive(Debug)]
pub enum ConnectionError {
    Io(std::io::Error),
    Protocol(crate::protocol::ProtocolError),
    Decode(crate::protocol::DecodeError),
    Timeout,
    Shutdown,
}

impl std::fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectionError::Io(e) => write!(f, "IO error: {}", e),
            ConnectionError::Protocol(e) => write!(f, "Protocol error: {}", e),
            ConnectionError::Decode(e) => write!(f, "Decode error: {}", e),
            ConnectionError::Timeout => write!(f, "Connection timeout"),
            ConnectionError::Shutdown => write!(f, "Shutdown"),
        }
    }
}

impl From<std::io::Error> for ConnectionError {
    fn from(e: std::io::Error) -> Self {
        ConnectionError::Io(e)
    }
}

impl From<crate::protocol::DecodeError> for ConnectionError {
    fn from(e: crate::protocol::DecodeError) -> Self {
        ConnectionError::Decode(e)
    }
}

/// Connection state
enum State {
    /// Waiting for CONNECT packet
    Connecting,
    /// Connected and running
    Connected {
        client_id: Arc<str>,
        session: Arc<RwLock<Session>>,
    },
}

/// Connection handler - generic over the stream type
pub struct Connection<S> {
    stream: S,
    addr: SocketAddr,
    state: State,
    decoder: Decoder,
    encoder: Encoder,
    read_buf: BytesMut,
    write_buf: BytesMut,
    sessions: Arc<SessionStore>,
    subscriptions: Arc<SubscriptionStore>,
    retained: Arc<DashMap<String, RetainedMessage>>,
    connections: Arc<DashMap<Arc<str>, mpsc::Sender<Packet>>>,
    config: BrokerConfig,
    events: broadcast::Sender<BrokerEvent>,
    packet_tx: mpsc::Sender<Packet>,
    packet_rx: mpsc::Receiver<Packet>,
    hooks: Arc<dyn Hooks>,
    metrics: Option<Arc<Metrics>>,
    /// Username from CONNECT packet (for ACL checks)
    username: Option<String>,
}

impl<S> Connection<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        stream: S,
        addr: SocketAddr,
        sessions: Arc<SessionStore>,
        subscriptions: Arc<SubscriptionStore>,
        retained: Arc<DashMap<String, RetainedMessage>>,
        connections: Arc<DashMap<Arc<str>, mpsc::Sender<Packet>>>,
        config: BrokerConfig,
        events: broadcast::Sender<BrokerEvent>,
        hooks: Arc<dyn Hooks>,
        metrics: Option<Arc<Metrics>>,
    ) -> Self {
        let (packet_tx, packet_rx) = mpsc::channel(1024);

        Self {
            stream,
            addr,
            state: State::Connecting,
            decoder: Decoder::new().with_max_packet_size(config.max_packet_size),
            encoder: Encoder::default(),
            read_buf: BytesMut::with_capacity(4096),
            write_buf: BytesMut::with_capacity(4096),
            sessions,
            subscriptions,
            retained,
            connections,
            config,
            events,
            packet_tx,
            packet_rx,
            hooks,
            metrics,
            username: None,
        }
    }

    /// Run the connection handler
    pub async fn run(&mut self) -> Result<(), ConnectionError> {
        // Wait for CONNECT packet with timeout
        let connect_timeout = Duration::from_secs(30);
        match timeout(connect_timeout, self.read_connect()).await {
            Ok(result) => result?,
            Err(_) => {
                debug!("Connect timeout from {}", self.addr);
                return Err(ConnectionError::Timeout);
            }
        }

        // Main loop
        self.run_connected().await
    }

    /// Read and process CONNECT packet
    async fn read_connect(&mut self) -> Result<(), ConnectionError> {
        loop {
            // Try to decode a packet from the buffer
            if let Some((packet, consumed)) = self.decoder.decode(&self.read_buf)? {
                self.read_buf.advance(consumed);

                match packet {
                    Packet::Connect(connect) => {
                        return self.handle_connect(*connect).await;
                    }
                    _ => {
                        // Protocol violation - first packet must be CONNECT
                        debug!("First packet from {} was not CONNECT", self.addr);
                        return Err(ConnectionError::Protocol(
                            crate::protocol::ProtocolError::ProtocolViolation(
                                "first packet must be CONNECT",
                            ),
                        ));
                    }
                }
            }

            // Read more data
            let n = self.stream.read_buf(&mut self.read_buf).await?;
            if n == 0 {
                return Err(ConnectionError::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "connection closed",
                )));
            }
        }
    }

    /// Handle CONNECT packet
    async fn handle_connect(&mut self, connect: Connect) -> Result<(), ConnectionError> {
        let protocol_version = connect.protocol_version;
        self.decoder.set_protocol_version(protocol_version);
        self.encoder.set_protocol_version(protocol_version);

        // Per MQTT-3.1.3-8: If client supplies zero-byte ClientId with CleanSession=0,
        // the server MUST respond with CONNACK return code 0x02 (Identifier rejected)
        if connect.client_id.is_empty() && !connect.clean_start {
            debug!(
                "Rejecting empty client ID with clean_start=false from {}",
                self.addr
            );
            let connack = ConnAck {
                session_present: false,
                reason_code: ReasonCode::ClientIdNotValid,
                properties: Properties::default(),
            };
            self.write_buf.clear();
            self.encoder
                .encode(&Packet::ConnAck(connack), &mut self.write_buf)
                .map_err(|e| ConnectionError::Protocol(e.into()))?;
            self.stream.write_all(&self.write_buf).await?;
            return Err(ConnectionError::Protocol(
                crate::protocol::ProtocolError::ProtocolViolation(
                    "empty client ID with clean_start=false",
                ),
            ));
        }

        // Validate client ID
        let client_id: Arc<str> = if connect.client_id.is_empty() {
            // Generate client ID (only allowed when clean_start=true)
            format!("roker-{:x}", rand_id()).into()
        } else {
            connect.client_id.clone().into()
        };

        debug!("CONNECT from {} (client_id: {})", self.addr, client_id);

        // Authenticate the client
        let auth_result = self
            .hooks
            .on_authenticate(
                &client_id,
                connect.username.as_deref(),
                connect.password.as_deref(),
            )
            .await;

        match auth_result {
            Ok(true) => {
                // Authentication successful, store username
                self.username = connect.username.clone();
                debug!("Authentication successful for {}", client_id);
            }
            Ok(false) => {
                debug!("Authentication failed for {}", client_id);
                let connack = ConnAck {
                    session_present: false,
                    reason_code: ReasonCode::NotAuthorized,
                    properties: Properties::default(),
                };
                self.write_buf.clear();
                self.encoder
                    .encode(&Packet::ConnAck(connack), &mut self.write_buf)
                    .map_err(|e| ConnectionError::Protocol(e.into()))?;
                self.stream.write_all(&self.write_buf).await?;
                return Err(ConnectionError::Protocol(
                    crate::protocol::ProtocolError::ProtocolViolation("authentication failed"),
                ));
            }
            Err(e) => {
                error!("Authentication error for {}: {}", client_id, e);
                let connack = ConnAck {
                    session_present: false,
                    reason_code: ReasonCode::UnspecifiedError,
                    properties: Properties::default(),
                };
                self.write_buf.clear();
                self.encoder
                    .encode(&Packet::ConnAck(connack), &mut self.write_buf)
                    .map_err(|e| ConnectionError::Protocol(e.into()))?;
                self.stream.write_all(&self.write_buf).await?;
                return Err(ConnectionError::Protocol(
                    crate::protocol::ProtocolError::ProtocolViolation("authentication error"),
                ));
            }
        }

        // Check for existing connection and disconnect it
        if let Some(existing) = self.connections.get(&client_id) {
            // Send disconnect to existing connection
            let disconnect = Packet::Disconnect(Disconnect {
                reason_code: ReasonCode::SessionTakenOver,
                properties: Properties::default(),
            });
            let _ = existing.try_send(disconnect);
        }

        // Get or create session
        let (session, session_present) =
            self.sessions
                .get_or_create(&client_id, protocol_version, connect.clean_start);

        // If clean_start=true, clear any previous subscriptions from the SubscriptionStore
        if connect.clean_start {
            self.subscriptions.unsubscribe_all(&client_id);
        }

        // Update session with connection parameters
        {
            let mut s = session.write();
            s.clean_start = connect.clean_start;
            s.keep_alive = if connect.keep_alive == 0 {
                self.config.default_keep_alive
            } else {
                connect.keep_alive.min(self.config.max_keep_alive)
            };

            // Handle session expiry based on protocol version
            if protocol_version == ProtocolVersion::V5 {
                // v5.0: use the session expiry interval from properties (defaults to 0 = delete on disconnect)
                if let Some(interval) = connect.properties.session_expiry_interval {
                    s.session_expiry_interval = interval;
                } else if !connect.clean_start {
                    // If clean_start=false but no expiry specified, use a reasonable default
                    s.session_expiry_interval = 0xFFFFFFFF; // Never expires
                }
                if let Some(max) = connect.properties.receive_maximum {
                    s.receive_maximum = max;
                    s.send_quota = max;
                }
                if let Some(max) = connect.properties.maximum_packet_size {
                    s.max_packet_size = max;
                }
                if let Some(max) = connect.properties.topic_alias_maximum {
                    s.topic_alias_maximum = max;
                }
            } else {
                // v3.1.1: clean_session=false means session persists indefinitely
                if !connect.clean_start {
                    s.session_expiry_interval = 0xFFFFFFFF; // Never expires
                } else {
                    s.session_expiry_interval = 0; // Delete on disconnect
                }
            }

            // Store will message
            if let Some(will) = connect.will {
                s.will = Some(WillMessage {
                    topic: will.topic,
                    payload: will.payload,
                    qos: will.qos,
                    retain: will.retain,
                    properties: will.properties.clone(),
                });
                if let Some(delay) = will.properties.will_delay_interval {
                    s.will_delay_interval = delay;
                }
            }

            s.touch();
        }

        // Register connection
        self.connections
            .insert(client_id.clone(), self.packet_tx.clone());

        // Send CONNACK
        let mut connack = ConnAck {
            session_present: session_present && !connect.clean_start,
            reason_code: ReasonCode::Success,
            properties: Properties::default(),
        };

        // Set v5.0 properties
        if protocol_version == ProtocolVersion::V5 {
            connack.properties.receive_maximum = Some(self.config.receive_maximum);
            // Per MQTT 5.0 spec 3.2.2.3.4: Maximum QoS can only be 0 or 1.
            // If server supports QoS 2, don't include this property (default is 2).
            if self.config.max_qos != QoS::ExactlyOnce {
                connack.properties.maximum_qos = Some(self.config.max_qos as u8);
            }
            connack.properties.retain_available =
                Some(if self.config.retain_available { 1 } else { 0 });
            connack.properties.maximum_packet_size = Some(self.config.max_packet_size as u32);
            connack.properties.topic_alias_maximum = Some(self.config.max_topic_alias);
            connack.properties.wildcard_subscription_available =
                Some(if self.config.wildcard_subscription_available {
                    1
                } else {
                    0
                });
            connack.properties.subscription_identifier_available =
                Some(if self.config.subscription_identifiers_available {
                    1
                } else {
                    0
                });
            connack.properties.shared_subscription_available =
                Some(if self.config.shared_subscriptions_available {
                    1
                } else {
                    0
                });

            // Assign client ID if we generated one
            if connect.client_id.is_empty() {
                connack.properties.assigned_client_identifier = Some(client_id.to_string());
            }
        }

        self.write_buf.clear();
        debug!("Encoding CONNACK for {}", client_id);
        self.encoder
            .encode(&Packet::ConnAck(connack), &mut self.write_buf)
            .map_err(|e| ConnectionError::Protocol(e.into()))?;
        debug!(
            "CONNACK encoded, {} bytes: {:02x?}",
            self.write_buf.len(),
            &self.write_buf[..]
        );
        self.stream.write_all(&self.write_buf).await?;
        debug!("CONNACK sent to {}", client_id);

        // Transition to connected state
        self.state = State::Connected {
            client_id: client_id.clone(),
            session: session.clone(),
        };

        // Notify event subscribers
        let _ = self.events.send(BrokerEvent::ClientConnected {
            client_id: client_id.clone(),
            protocol_version,
        });

        // Send pending messages (respecting send quota per MQTT-4.9.0-2
        // and max packet size per MQTT-3.1.2-24)
        {
            let (pending, max_packet_size) = {
                let mut s = session.write();
                (s.drain_pending_messages(), s.max_packet_size)
            };

            for mut publish in pending {
                if publish.qos != QoS::AtMostOnce {
                    let mut s = session.write();
                    if !s.decrement_send_quota() {
                        // Quota exhausted - re-queue remaining messages
                        if s.queue_message(publish) == QueueResult::DroppedOldest {
                            let _ = self.events.send(BrokerEvent::MessageDropped);
                        }
                        continue;
                    }
                    publish.packet_id = Some(s.next_packet_id());
                    // Store inflight for retry
                    if let Some(packet_id) = publish.packet_id {
                        s.inflight_outgoing.insert(
                            packet_id,
                            InflightMessage {
                                packet_id,
                                publish: publish.clone(),
                                qos2_state: if publish.qos == QoS::ExactlyOnce {
                                    Some(Qos2State::WaitingPubRec)
                                } else {
                                    None
                                },
                                sent_at: Instant::now(),
                                retry_count: 0,
                            },
                        );
                    }
                }

                self.write_buf.clear();
                self.encoder
                    .encode(&Packet::Publish(publish), &mut self.write_buf)
                    .map_err(|e| ConnectionError::Protocol(e.into()))?;

                // Per MQTT v5.0 spec [MQTT-3.1.2-24]: MUST NOT send packets
                // exceeding client's Maximum Packet Size
                if self.write_buf.len() > max_packet_size as usize {
                    warn!(
                        "Dropping pending PUBLISH: encoded size {} exceeds client max {}",
                        self.write_buf.len(),
                        max_packet_size
                    );
                    continue;
                }

                let bytes_sent = self.write_buf.len();
                self.stream.write_all(&self.write_buf).await?;
                if let Some(ref metrics) = self.metrics {
                    metrics.publish_sent(bytes_sent);
                }
            }
        }

        // Send retained messages for existing subscriptions
        if session_present {
            let subs: Vec<_> = {
                let s = session.read();
                s.subscriptions.values().cloned().collect()
            };

            for sub in subs {
                self.send_retained_messages(
                    &client_id,
                    &sub.filter,
                    sub.options.qos,
                    &session,
                    sub.subscription_id,
                )
                .await?;
            }
        }

        Ok(())
    }

    /// Run the main connection loop
    async fn run_connected(&mut self) -> Result<(), ConnectionError> {
        let (client_id, session) = match &self.state {
            State::Connected { client_id, session } => (client_id.clone(), session.clone()),
            _ => {
                return Err(ConnectionError::Protocol(
                    crate::protocol::ProtocolError::ProtocolViolation("not connected"),
                ))
            }
        };

        let keep_alive = {
            let s = session.read();
            Duration::from_secs((s.keep_alive as u64 * 3) / 2)
        };

        loop {
            tokio::select! {
                // Read from socket
                result = self.stream.read_buf(&mut self.read_buf) => {
                    match result {
                        Ok(0) => {
                            // Connection closed
                            debug!("Connection closed from {}", self.addr);
                            self.handle_disconnect(&client_id, &session, true).await;
                            return Ok(());
                        }
                        Ok(_) => {
                            // Process packets
                            while let Some((packet, consumed)) = self.decoder.decode(&self.read_buf)? {
                                self.read_buf.advance(consumed);

                                // Update activity timestamp
                                {
                                    let mut s = session.write();
                                    s.touch();
                                }

                                if let Err(e) = self.handle_packet(&client_id, &session, packet).await {
                                    match &e {
                                        ConnectionError::Shutdown => {
                                            // Normal disconnect, already handled in handle_packet
                                            return Err(e);
                                        }
                                        _ => {
                                            error!("Error handling packet: {}", e);
                                            self.handle_disconnect(&client_id, &session, true).await;
                                            return Err(e);
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            error!("Read error: {}", e);
                            self.handle_disconnect(&client_id, &session, true).await;
                            return Err(e.into());
                        }
                    }
                }

                // Receive packets to send
                Some(packet) = self.packet_rx.recv() => {
                    match packet {
                        Packet::Disconnect(_) => {
                            // We're being disconnected (session takeover)
                            self.write_buf.clear();
                            let _ = self.encoder.encode(&packet, &mut self.write_buf);
                            let _ = self.stream.write_all(&self.write_buf).await;
                            return Ok(());
                        }
                        Packet::Publish(mut publish) => {
                            // Get max packet size from session
                            let max_packet_size = {
                                let s = session.read();
                                s.max_packet_size
                            };

                            // Per MQTT v5.0 spec [MQTT-4.9.0-2]: MUST NOT send QoS>0
                            // PUBLISH when send quota is 0
                            if publish.qos != QoS::AtMostOnce {
                                let mut s = session.write();
                                if !s.decrement_send_quota() {
                                    // Quota exhausted - queue message for later delivery
                                    debug!(
                                        "Send quota exhausted for {}, queuing message",
                                        s.client_id
                                    );
                                    if s.queue_message(publish) == QueueResult::DroppedOldest {
                                        let _ = self.events.send(BrokerEvent::MessageDropped);
                                    }
                                    continue;
                                }
                                // Assign packet ID
                                if publish.packet_id.is_none() {
                                    publish.packet_id = Some(s.next_packet_id());
                                }
                                // Store inflight
                                if let Some(packet_id) = publish.packet_id {
                                    s.inflight_outgoing.insert(packet_id, InflightMessage {
                                        packet_id,
                                        publish: publish.clone(),
                                        qos2_state: if publish.qos == QoS::ExactlyOnce {
                                            Some(Qos2State::WaitingPubRec)
                                        } else {
                                            None
                                        },
                                        sent_at: Instant::now(),
                                        retry_count: 0,
                                    });
                                }
                            }

                            self.write_buf.clear();
                            self.encoder.encode(&Packet::Publish(publish), &mut self.write_buf)
                                .map_err(|e| ConnectionError::Protocol(e.into()))?;

                            // Per MQTT v5.0 spec [MQTT-3.1.2-24]: MUST NOT send packets
                            // exceeding client's Maximum Packet Size
                            if self.write_buf.len() > max_packet_size as usize {
                                warn!(
                                    "Dropping PUBLISH: encoded size {} exceeds client max {}",
                                    self.write_buf.len(),
                                    max_packet_size
                                );
                                continue;
                            }

                            let bytes_sent = self.write_buf.len();
                            self.stream.write_all(&self.write_buf).await?;
                            if let Some(ref metrics) = self.metrics {
                                metrics.publish_sent(bytes_sent);
                            }
                        }
                        _ => {
                            self.write_buf.clear();
                            self.encoder.encode(&packet, &mut self.write_buf)
                                .map_err(|e| ConnectionError::Protocol(e.into()))?;
                            self.stream.write_all(&self.write_buf).await?;
                        }
                    }
                }

                // Keep alive timeout
                _ = tokio::time::sleep(keep_alive) => {
                    let expired = {
                        let s = session.read();
                        s.is_keep_alive_expired()
                    };
                    if expired {
                        debug!("Keep alive timeout for {}", client_id);
                        self.handle_disconnect(&client_id, &session, true).await;
                        return Err(ConnectionError::Timeout);
                    }
                }
            }
        }
    }

    /// Handle an incoming packet
    async fn handle_packet(
        &mut self,
        client_id: &Arc<str>,
        session: &Arc<RwLock<Session>>,
        packet: Packet,
    ) -> Result<(), ConnectionError> {
        match packet {
            Packet::Connect(_) => {
                // Protocol violation - CONNECT already received
                Err(ConnectionError::Protocol(
                    crate::protocol::ProtocolError::ProtocolViolation("duplicate CONNECT"),
                ))
            }
            Packet::Publish(publish) => self.handle_publish(client_id, session, publish).await,
            Packet::PubAck(puback) => self.handle_puback(session, puback).await,
            Packet::PubRec(pubrec) => self.handle_pubrec(session, pubrec).await,
            Packet::PubRel(pubrel) => self.handle_pubrel(client_id, session, pubrel).await,
            Packet::PubComp(pubcomp) => self.handle_pubcomp(session, pubcomp).await,
            Packet::Subscribe(subscribe) => {
                self.handle_subscribe(client_id, session, subscribe).await
            }
            Packet::Unsubscribe(unsubscribe) => {
                self.handle_unsubscribe(client_id, session, unsubscribe)
                    .await
            }
            Packet::PingReq => {
                self.write_buf.clear();
                self.encoder
                    .encode(&Packet::PingResp, &mut self.write_buf)
                    .map_err(|e| ConnectionError::Protocol(e.into()))?;
                self.stream.write_all(&self.write_buf).await?;
                Ok(())
            }
            Packet::Disconnect(disconnect) => {
                debug!(
                    "DISCONNECT from {} (reason: {:?})",
                    client_id, disconnect.reason_code
                );
                // Per MQTT v5.0 spec [MQTT-3.1.2-10]:
                // - Reason 0x00 (Normal): will message MUST be deleted, NOT published
                // - Reason 0x04 (DisconnectWithWill): will message MUST still be published
                let publish_will =
                    disconnect.reason_code == crate::protocol::ReasonCode::DisconnectWithWill;
                self.handle_disconnect(client_id, session, publish_will)
                    .await;
                Err(ConnectionError::Shutdown)
            }
            _ => {
                // Unexpected packet type
                warn!(
                    "Unexpected packet type from {}: {:?}",
                    client_id,
                    packet.packet_type()
                );
                Ok(())
            }
        }
    }

    /// Handle PUBLISH packet
    async fn handle_publish(
        &mut self,
        client_id: &Arc<str>,
        session: &Arc<RwLock<Session>>,
        mut publish: Publish,
    ) -> Result<(), ConnectionError> {
        // Validate topic name
        if let Err(e) = validate_topic_name(&publish.topic) {
            warn!("Invalid topic name from {}: {}", client_id, e);
            // For v5.0, send PUBACK/PUBREC with error
            if publish.qos != QoS::AtMostOnce {
                let packet_id = publish.packet_id.unwrap();
                let response = if publish.qos == QoS::AtLeastOnce {
                    Packet::PubAck(PubAck {
                        packet_id,
                        reason_code: ReasonCode::TopicNameInvalid,
                        properties: Properties::default(),
                    })
                } else {
                    Packet::PubRec(PubRec {
                        packet_id,
                        reason_code: ReasonCode::TopicNameInvalid,
                        properties: Properties::default(),
                    })
                };
                self.write_buf.clear();
                self.encoder
                    .encode(&response, &mut self.write_buf)
                    .map_err(|e| ConnectionError::Protocol(e.into()))?;
                self.stream.write_all(&self.write_buf).await?;
            }
            return Ok(());
        }

        // Handle topic alias (v5.0)
        if let Some(alias) = publish.properties.topic_alias {
            if publish.topic.is_empty() {
                // Lookup alias
                let s = session.read();
                if let Some(topic) = s.resolve_topic_alias(alias) {
                    publish.topic = topic.clone();
                } else {
                    // Invalid alias
                    return Err(ConnectionError::Protocol(
                        crate::protocol::ProtocolError::ProtocolViolation("unknown topic alias"),
                    ));
                }
            } else {
                // Register alias
                let mut s = session.write();
                s.register_topic_alias(alias, publish.topic.clone());
            }
        }

        trace!(
            "PUBLISH from {} to {} (QoS {:?})",
            client_id,
            publish.topic,
            publish.qos
        );

        // Check ACL for publish permission
        let acl_result = self
            .hooks
            .on_publish_check(
                client_id,
                self.username.as_deref(),
                &publish.topic,
                publish.qos,
                publish.retain,
            )
            .await;

        match acl_result {
            Ok(true) => {
                // Publish allowed
            }
            Ok(false) => {
                debug!(
                    "PUBLISH denied for {} to topic {} (ACL)",
                    client_id, publish.topic
                );
                // For QoS > 0, send acknowledgment with error reason code
                if publish.qos != QoS::AtMostOnce {
                    let packet_id = publish.packet_id.unwrap();
                    let response = if publish.qos == QoS::AtLeastOnce {
                        Packet::PubAck(PubAck {
                            packet_id,
                            reason_code: ReasonCode::NotAuthorized,
                            properties: Properties::default(),
                        })
                    } else {
                        Packet::PubRec(PubRec {
                            packet_id,
                            reason_code: ReasonCode::NotAuthorized,
                            properties: Properties::default(),
                        })
                    };
                    self.write_buf.clear();
                    self.encoder
                        .encode(&response, &mut self.write_buf)
                        .map_err(|e| ConnectionError::Protocol(e.into()))?;
                    self.stream.write_all(&self.write_buf).await?;
                }
                return Ok(());
            }
            Err(e) => {
                error!("ACL check error for {}: {}", client_id, e);
                // For QoS > 0, send error acknowledgment
                if publish.qos != QoS::AtMostOnce {
                    let packet_id = publish.packet_id.unwrap();
                    let response = if publish.qos == QoS::AtLeastOnce {
                        Packet::PubAck(PubAck {
                            packet_id,
                            reason_code: ReasonCode::UnspecifiedError,
                            properties: Properties::default(),
                        })
                    } else {
                        Packet::PubRec(PubRec {
                            packet_id,
                            reason_code: ReasonCode::UnspecifiedError,
                            properties: Properties::default(),
                        })
                    };
                    self.write_buf.clear();
                    self.encoder
                        .encode(&response, &mut self.write_buf)
                        .map_err(|e| ConnectionError::Protocol(e.into()))?;
                    self.stream.write_all(&self.write_buf).await?;
                }
                return Ok(());
            }
        }

        // Handle QoS
        match publish.qos {
            QoS::AtMostOnce => {
                // No acknowledgment needed
            }
            QoS::AtLeastOnce => {
                // Send PUBACK
                let puback = PubAck::new(publish.packet_id.unwrap());
                self.write_buf.clear();
                self.encoder
                    .encode(&Packet::PubAck(puback), &mut self.write_buf)
                    .map_err(|e| ConnectionError::Protocol(e.into()))?;
                self.stream.write_all(&self.write_buf).await?;
            }
            QoS::ExactlyOnce => {
                // Store message and send PUBREC - message will be routed on PUBREL
                let packet_id = publish.packet_id.unwrap();
                {
                    let mut s = session.write();
                    s.inflight_incoming.insert(packet_id, publish.clone());
                }

                let pubrec = PubRec::new(packet_id);
                self.write_buf.clear();
                self.encoder
                    .encode(&Packet::PubRec(pubrec), &mut self.write_buf)
                    .map_err(|e| ConnectionError::Protocol(e.into()))?;
                self.stream.write_all(&self.write_buf).await?;

                // For QoS 2, we route after PUBREL (not now)
                // Handle retained message now, but don't route to subscribers yet
                if publish.retain && self.config.retain_available {
                    if publish.payload.is_empty() {
                        self.retained.remove(&publish.topic);
                    } else {
                        self.retained.insert(
                            publish.topic.clone(),
                            RetainedMessage {
                                topic: publish.topic.clone(),
                                payload: publish.payload.clone(),
                                qos: publish.qos,
                                properties: publish.properties.clone(),
                                timestamp: Instant::now(),
                            },
                        );
                    }
                }
                return Ok(());
            }
        }

        // Handle retained message
        if publish.retain && self.config.retain_available {
            if publish.payload.is_empty() {
                self.retained.remove(&publish.topic);
            } else {
                self.retained.insert(
                    publish.topic.clone(),
                    RetainedMessage {
                        topic: publish.topic.clone(),
                        payload: publish.payload.clone(),
                        qos: publish.qos,
                        properties: publish.properties.clone(),
                        timestamp: Instant::now(),
                    },
                );
            }
        }

        // Route message to subscribers
        self.route_message(client_id, &publish).await?;

        Ok(())
    }

    /// Route a message to subscribers
    /// Performance: Uses AHashMap for deduplication and SmallVec for subscription IDs
    async fn route_message(
        &self,
        sender_id: &Arc<str>,
        publish: &Publish,
    ) -> Result<(), ConnectionError> {
        let matches = self.subscriptions.matches(&publish.topic);

        // Deduplicate by client_id, keeping highest QoS and collecting ALL subscription IDs
        // Uses SmallVec for subscription_ids since most messages match few subscriptions
        struct ClientSub {
            qos: QoS,
            retain_as_published: bool,
            subscription_ids: SmallVec<[u32; 4]>,
        }
        // Use AHashMap for faster hashing, pre-allocate for typical workload
        let mut client_subs: AHashMap<Arc<str>, ClientSub> = AHashMap::with_capacity(matches.len());
        for sub in matches {
            // Skip sender if no_local is set
            if sub.no_local && sub.client_id == *sender_id {
                continue;
            }

            let entry = client_subs
                .entry(sub.client_id.clone())
                .or_insert(ClientSub {
                    qos: QoS::AtMostOnce,
                    retain_as_published: false,
                    subscription_ids: SmallVec::new(),
                });

            // Update QoS to highest
            if sub.qos > entry.qos {
                entry.qos = sub.qos;
            }

            // If ANY matching subscription has retain_as_published=true, preserve retain flag
            if sub.retain_as_published {
                entry.retain_as_published = true;
            }

            // Collect ALL subscription identifiers
            if let Some(id) = sub.subscription_id {
                if !entry.subscription_ids.contains(&id) {
                    entry.subscription_ids.push(id);
                }
            }
        }

        // Send to each client
        for (client_id, sub_info) in client_subs {
            let effective_qos = publish.qos.min(sub_info.qos);

            let mut outgoing = publish.clone();
            outgoing.qos = effective_qos;
            outgoing.dup = false;
            // Clear incoming packet_id - broker assigns fresh IDs for each subscriber
            outgoing.packet_id = None;

            // Clear retain flag unless retain_as_published
            if !sub_info.retain_as_published {
                outgoing.retain = false;
            }

            // Add ALL subscription identifiers
            for id in sub_info.subscription_ids {
                outgoing.properties.subscription_identifiers.push(id);
            }

            if let Some(sender) = self.connections.get(&client_id) {
                let _ = sender.try_send(Packet::Publish(outgoing));
            } else {
                // Client disconnected, queue message if persistent session
                if let Some(session) = self.sessions.get(client_id.as_ref()) {
                    let mut s = session.write();
                    if !s.clean_start && s.queue_message(outgoing) == QueueResult::DroppedOldest {
                        let _ = self.events.send(BrokerEvent::MessageDropped);
                    }
                }
            }
        }

        // Notify event subscribers (for bridge forwarding and monitoring)
        let _ = self.events.send(BrokerEvent::MessagePublished {
            topic: publish.topic.clone(),
            payload: publish.payload.clone(),
            qos: publish.qos,
            retain: publish.retain,
        });

        Ok(())
    }

    /// Handle PUBACK packet
    async fn handle_puback(
        &mut self,
        session: &Arc<RwLock<Session>>,
        puback: PubAck,
    ) -> Result<(), ConnectionError> {
        let mut s = session.write();
        s.inflight_outgoing.remove(&puback.packet_id);
        s.increment_send_quota();
        Ok(())
    }

    /// Handle PUBREC packet
    async fn handle_pubrec(
        &mut self,
        session: &Arc<RwLock<Session>>,
        pubrec: PubRec,
    ) -> Result<(), ConnectionError> {
        {
            let mut s = session.write();
            if let Some(inflight) = s.inflight_outgoing.get_mut(&pubrec.packet_id) {
                inflight.qos2_state = Some(Qos2State::WaitingPubComp);
            }
        }

        // Send PUBREL
        let pubrel = PubRel::new(pubrec.packet_id);
        self.write_buf.clear();
        self.encoder
            .encode(&Packet::PubRel(pubrel), &mut self.write_buf)
            .map_err(|e| ConnectionError::Protocol(e.into()))?;
        self.stream.write_all(&self.write_buf).await?;

        Ok(())
    }

    /// Handle PUBREL packet
    async fn handle_pubrel(
        &mut self,
        client_id: &Arc<str>,
        session: &Arc<RwLock<Session>>,
        pubrel: PubRel,
    ) -> Result<(), ConnectionError> {
        // Get the stored message
        let publish = {
            let mut s = session.write();
            s.inflight_incoming.remove(&pubrel.packet_id)
        };

        // Send PUBCOMP
        let pubcomp = PubComp::new(pubrel.packet_id);
        self.write_buf.clear();
        self.encoder
            .encode(&Packet::PubComp(pubcomp), &mut self.write_buf)
            .map_err(|e| ConnectionError::Protocol(e.into()))?;
        self.stream.write_all(&self.write_buf).await?;

        // Now route the message to subscribers (QoS 2 delivery complete)
        if let Some(publish) = publish {
            self.route_message(client_id, &publish).await?;
        }

        Ok(())
    }

    /// Handle PUBCOMP packet
    async fn handle_pubcomp(
        &mut self,
        session: &Arc<RwLock<Session>>,
        pubcomp: PubComp,
    ) -> Result<(), ConnectionError> {
        let mut s = session.write();
        s.inflight_outgoing.remove(&pubcomp.packet_id);
        s.increment_send_quota();
        Ok(())
    }

    /// Handle SUBSCRIBE packet
    async fn handle_subscribe(
        &mut self,
        client_id: &Arc<str>,
        session: &Arc<RwLock<Session>>,
        subscribe: Subscribe,
    ) -> Result<(), ConnectionError> {
        let mut reason_codes = Vec::with_capacity(subscribe.subscriptions.len());
        let _protocol_version = self
            .decoder
            .protocol_version()
            .unwrap_or(ProtocolVersion::V5);

        // Get subscription identifier from properties
        let sub_id = subscribe
            .properties
            .subscription_identifiers
            .first()
            .copied();

        // Track subscription info for retained message handling
        let mut sub_info: Vec<(QoS, bool, RetainHandling, String)> = Vec::new();

        for sub in &subscribe.subscriptions {
            // Validate topic filter
            if validate_topic_filter(&sub.filter).is_err() {
                reason_codes.push(ReasonCode::TopicFilterInvalid);
                sub_info.push((
                    QoS::AtMostOnce,
                    false,
                    RetainHandling::DoNotSend,
                    sub.filter.clone(),
                ));
                continue;
            }

            // Check wildcard support
            if !self.config.wildcard_subscription_available
                && (sub.filter.contains('+') || sub.filter.contains('#'))
            {
                reason_codes.push(ReasonCode::WildcardSubsNotSupported);
                sub_info.push((
                    QoS::AtMostOnce,
                    false,
                    RetainHandling::DoNotSend,
                    sub.filter.clone(),
                ));
                continue;
            }

            // Check ACL for subscribe permission
            let acl_result = self
                .hooks
                .on_subscribe_check(
                    client_id,
                    self.username.as_deref(),
                    &sub.filter,
                    sub.options.qos,
                )
                .await;

            match acl_result {
                Ok(true) => {
                    // Subscribe allowed
                }
                Ok(false) => {
                    debug!(
                        "SUBSCRIBE denied for {} to filter {} (ACL)",
                        client_id, sub.filter
                    );
                    reason_codes.push(ReasonCode::NotAuthorized);
                    sub_info.push((
                        QoS::AtMostOnce,
                        false,
                        RetainHandling::DoNotSend,
                        sub.filter.clone(),
                    ));
                    continue;
                }
                Err(e) => {
                    error!("ACL check error for {}: {}", client_id, e);
                    reason_codes.push(ReasonCode::UnspecifiedError);
                    sub_info.push((
                        QoS::AtMostOnce,
                        false,
                        RetainHandling::DoNotSend,
                        sub.filter.clone(),
                    ));
                    continue;
                }
            }

            // Check QoS support
            let granted_qos = sub.options.qos.min(self.config.max_qos);

            // Check if subscription already existed (for retain_handling=1)
            let subscription_existed = {
                let s = session.read();
                s.subscriptions.contains_key(sub.filter.as_str())
            };

            // Add subscription (SubscriptionStore handles $share parsing internally)
            self.subscriptions.subscribe(
                &sub.filter,
                Subscription {
                    client_id: client_id.clone(),
                    qos: granted_qos,
                    no_local: sub.options.no_local,
                    retain_as_published: sub.options.retain_as_published,
                    subscription_id: sub_id,
                    share_group: None, // Will be set by SubscriptionStore if this is a shared subscription
                },
            );

            // Store in session
            {
                let mut s = session.write();
                s.add_subscription(sub.filter.clone(), sub.options, sub_id);
            }

            // Track info for retained message handling
            sub_info.push((
                granted_qos,
                subscription_existed,
                sub.options.retain_handling,
                sub.filter.clone(),
            ));

            // Return granted QoS
            reason_codes.push(match granted_qos {
                QoS::AtMostOnce => ReasonCode::Success,
                QoS::AtLeastOnce => ReasonCode::GrantedQoS1,
                QoS::ExactlyOnce => ReasonCode::GrantedQoS2,
            });

            // Emit subscription event for cluster synchronization
            let _ = self.events.send(BrokerEvent::SubscriptionAdded {
                filter: sub.filter.clone(),
                client_id: client_id.clone(),
            });

            debug!(
                "SUBSCRIBE {} to {} (QoS {:?})",
                client_id, sub.filter, granted_qos
            );
        }

        // Send SUBACK
        let suback = SubAck {
            packet_id: subscribe.packet_id,
            reason_codes: reason_codes.clone(),
            properties: Properties::default(),
        };

        self.write_buf.clear();
        self.encoder
            .encode(&Packet::SubAck(suback), &mut self.write_buf)
            .map_err(|e| ConnectionError::Protocol(e.into()))?;
        self.stream.write_all(&self.write_buf).await?;

        // Send retained messages based on retain_handling option
        for ((granted_qos, existed, retain_handling, filter), reason) in
            sub_info.iter().zip(reason_codes.iter())
        {
            if !reason.is_success() {
                continue;
            }

            // Check retain_handling option
            let should_send = match retain_handling {
                RetainHandling::SendAtSubscribe => true,
                RetainHandling::SendAtSubscribeIfNew => !existed,
                RetainHandling::DoNotSend => false,
            };

            if should_send {
                self.send_retained_messages(client_id, filter, *granted_qos, session, sub_id)
                    .await?;
            }
        }

        Ok(())
    }

    /// Send retained messages for a subscription
    async fn send_retained_messages(
        &mut self,
        _client_id: &Arc<str>,
        filter: &str,
        qos: QoS,
        session: &Arc<RwLock<Session>>,
        subscription_id: Option<u32>,
    ) -> Result<(), ConnectionError> {
        // Find matching retained messages
        let mut matching_retained = Vec::new();
        for entry in self.retained.iter() {
            if crate::topic::validation::topic_matches_filter(entry.key(), filter) {
                matching_retained.push(entry.clone());
            }
        }

        for retained in matching_retained {
            // Calculate elapsed time for message expiry countdown
            let elapsed_secs = retained.timestamp.elapsed().as_secs() as u32;

            // Check if message has expired
            if let Some(expiry) = retained.properties.message_expiry_interval {
                if elapsed_secs >= expiry {
                    // Message has expired, skip it
                    continue;
                }
            }

            let effective_qos = retained.qos.min(qos);

            let mut publish = Publish {
                dup: false,
                qos: effective_qos,
                retain: true,
                topic: retained.topic.clone(),
                packet_id: None,
                payload: retained.payload.clone(),
                properties: retained.properties.clone(),
            };

            // Decrement message expiry interval by elapsed time
            if let Some(expiry) = publish.properties.message_expiry_interval {
                publish.properties.message_expiry_interval =
                    Some(expiry.saturating_sub(elapsed_secs));
            }

            // Add subscription identifier
            if let Some(sub_id) = subscription_id {
                publish.properties.subscription_identifiers.push(sub_id);
            }

            if effective_qos != QoS::AtMostOnce {
                let mut s = session.write();
                publish.packet_id = Some(s.next_packet_id());
            }

            self.write_buf.clear();
            self.encoder
                .encode(&Packet::Publish(publish), &mut self.write_buf)
                .map_err(|e| ConnectionError::Protocol(e.into()))?;
            let bytes_sent = self.write_buf.len();
            self.stream.write_all(&self.write_buf).await?;
            if let Some(ref metrics) = self.metrics {
                metrics.publish_sent(bytes_sent);
            }
        }

        Ok(())
    }

    /// Handle UNSUBSCRIBE packet
    async fn handle_unsubscribe(
        &mut self,
        client_id: &Arc<str>,
        session: &Arc<RwLock<Session>>,
        unsubscribe: Unsubscribe,
    ) -> Result<(), ConnectionError> {
        let mut reason_codes = Vec::with_capacity(unsubscribe.filters.len());
        let protocol_version = self
            .decoder
            .protocol_version()
            .unwrap_or(ProtocolVersion::V5);

        for filter in &unsubscribe.filters {
            let removed = self.subscriptions.unsubscribe(filter, client_id);

            // Remove from session
            {
                let mut s = session.write();
                s.remove_subscription(filter);
            }

            if protocol_version == ProtocolVersion::V5 {
                reason_codes.push(if removed {
                    ReasonCode::Success
                } else {
                    ReasonCode::NoSubscriptionExisted
                });
            }

            // Emit unsubscription event for cluster synchronization
            if removed {
                let _ = self.events.send(BrokerEvent::SubscriptionRemoved {
                    filter: filter.clone(),
                    client_id: client_id.clone(),
                });
            }

            debug!("UNSUBSCRIBE {} from {}", client_id, filter);
        }

        // Send UNSUBACK
        let unsuback = UnsubAck {
            packet_id: unsubscribe.packet_id,
            reason_codes,
            properties: Properties::default(),
        };

        self.write_buf.clear();
        self.encoder
            .encode(&Packet::UnsubAck(unsuback), &mut self.write_buf)
            .map_err(|e| ConnectionError::Protocol(e.into()))?;
        self.stream.write_all(&self.write_buf).await?;

        Ok(())
    }

    /// Handle disconnection
    async fn handle_disconnect(
        &mut self,
        client_id: &Arc<str>,
        session: &Arc<RwLock<Session>>,
        publish_will: bool,
    ) {
        // Remove from connections
        self.connections.remove(client_id);

        // Remove subscriptions if clean start
        let (clean_start, will, will_delay_interval) = {
            let s = session.read();
            (s.clean_start, s.will.clone(), s.will_delay_interval)
        };

        if clean_start {
            self.subscriptions.unsubscribe_all(client_id);
        }

        // Mark session as disconnected
        self.sessions.disconnect(client_id);

        // Publish will message if needed
        if publish_will {
            if let Some(will) = will {
                if will_delay_interval > 0 {
                    // Spawn delayed will publish task
                    let session = session.clone();
                    let client_id = client_id.clone();
                    let retained = self.retained.clone();
                    let subscriptions = self.subscriptions.clone();
                    let connections = self.connections.clone();
                    let sessions = self.sessions.clone();
                    let config = self.config.clone();
                    let events = self.events.clone();
                    let delay = Duration::from_secs(will_delay_interval as u64);

                    // Capture the disconnect timestamp to detect reconnect+disconnect cycles
                    let disconnected_at = {
                        let s = session.read();
                        s.disconnected_at
                    };

                    tokio::spawn(async move {
                        tokio::time::sleep(delay).await;

                        // Check if:
                        // 1. This session is still the active session (not replaced by clean_start=true)
                        // 2. Session is still disconnected from the SAME disconnect event
                        // 3. Will is still pending
                        let should_publish = {
                            // First check if this session is still the active one in the store
                            let is_current_session = sessions
                                .get(client_id.as_ref())
                                .map(|s| Arc::ptr_eq(&s, &session))
                                .unwrap_or(false);

                            if !is_current_session {
                                false
                            } else {
                                let s = session.read();
                                s.state == crate::session::SessionState::Disconnected
                                    && s.will.is_some()
                                    && s.disconnected_at == disconnected_at
                            }
                        };

                        if should_publish {
                            // Take the will from the session
                            let will = {
                                let mut s = session.write();
                                s.will.take()
                            };

                            if let Some(will) = will {
                                debug!(
                                    "Publishing delayed will message for {} to {}",
                                    client_id, will.topic
                                );

                                let publish = Publish {
                                    dup: false,
                                    qos: will.qos,
                                    retain: will.retain,
                                    topic: will.topic.clone(),
                                    packet_id: None,
                                    payload: will.payload,
                                    properties: will.properties,
                                };

                                // Handle retained
                                if will.retain && config.retain_available {
                                    if publish.payload.is_empty() {
                                        retained.remove(&will.topic);
                                    } else {
                                        retained.insert(
                                            will.topic.clone(),
                                            RetainedMessage {
                                                topic: will.topic.clone(),
                                                payload: publish.payload.clone(),
                                                qos: publish.qos,
                                                properties: publish.properties.clone(),
                                                timestamp: Instant::now(),
                                            },
                                        );
                                    }
                                }

                                // Route will message to subscribers
                                let _ = route_will_message(
                                    &subscriptions,
                                    &connections,
                                    &sessions,
                                    &events,
                                    &client_id,
                                    &publish,
                                )
                                .await;
                            }
                        } else {
                            debug!(
                                "Skipping delayed will for {} (client reconnected or will cleared)",
                                client_id
                            );
                        }
                    });
                } else {
                    // Publish immediately (no delay)
                    let publish = Publish {
                        dup: false,
                        qos: will.qos,
                        retain: will.retain,
                        topic: will.topic.clone(),
                        packet_id: None,
                        payload: will.payload,
                        properties: will.properties,
                    };

                    // Handle retained
                    if will.retain && self.config.retain_available {
                        if publish.payload.is_empty() {
                            self.retained.remove(&will.topic);
                        } else {
                            self.retained.insert(
                                will.topic.clone(),
                                RetainedMessage {
                                    topic: will.topic.clone(),
                                    payload: publish.payload.clone(),
                                    qos: publish.qos,
                                    properties: publish.properties.clone(),
                                    timestamp: Instant::now(),
                                },
                            );
                        }
                    }

                    // Route will message
                    let _ = self.route_message(client_id, &publish).await;

                    // Clear will from session (only when publishing immediately)
                    {
                        let mut s = session.write();
                        s.will = None;
                    }
                }
            }
        } else {
            // Normal disconnect - clear will from session
            let mut s = session.write();
            s.will = None;
        }

        // Notify event subscribers
        let _ = self.events.send(BrokerEvent::ClientDisconnected {
            client_id: client_id.clone(),
        });

        debug!("Client {} disconnected", client_id);
    }
}

/// Generate a random ID
fn rand_id() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    let hasher = RandomState::new().build_hasher();
    hasher.finish()
}

// Extension trait for BytesMut to add advance method
trait BytesMutExt {
    fn advance(&mut self, cnt: usize);
}

impl BytesMutExt for BytesMut {
    fn advance(&mut self, cnt: usize) {
        bytes::Buf::advance(self, cnt);
    }
}

/// Route a will message to subscribers (standalone function for delayed will tasks)
/// Performance: Uses AHashMap for deduplication and SmallVec for subscription IDs
async fn route_will_message(
    subscriptions: &SubscriptionStore,
    connections: &DashMap<Arc<str>, mpsc::Sender<Packet>>,
    sessions: &SessionStore,
    events: &broadcast::Sender<BrokerEvent>,
    sender_id: &Arc<str>,
    publish: &Publish,
) -> Result<(), ConnectionError> {
    let matches = subscriptions.matches(&publish.topic);

    // Deduplicate by client_id, keeping highest QoS and collecting ALL subscription IDs
    struct ClientSub {
        qos: QoS,
        retain_as_published: bool,
        subscription_ids: SmallVec<[u32; 4]>,
    }
    let mut client_subs: AHashMap<Arc<str>, ClientSub> = AHashMap::with_capacity(matches.len());
    for sub in matches {
        // Skip sender if no_local is set
        if sub.no_local && sub.client_id == *sender_id {
            continue;
        }

        let entry = client_subs
            .entry(sub.client_id.clone())
            .or_insert(ClientSub {
                qos: QoS::AtMostOnce,
                retain_as_published: false,
                subscription_ids: SmallVec::new(),
            });

        // Update QoS to highest
        if sub.qos > entry.qos {
            entry.qos = sub.qos;
        }

        // If ANY matching subscription has retain_as_published=true, preserve retain flag
        if sub.retain_as_published {
            entry.retain_as_published = true;
        }

        // Collect ALL subscription identifiers
        if let Some(id) = sub.subscription_id {
            if !entry.subscription_ids.contains(&id) {
                entry.subscription_ids.push(id);
            }
        }
    }

    // Send to each client
    for (client_id, sub_info) in client_subs {
        let effective_qos = publish.qos.min(sub_info.qos);

        let mut outgoing = publish.clone();
        outgoing.qos = effective_qos;
        outgoing.dup = false;
        // Clear incoming packet_id - broker assigns fresh IDs for each subscriber
        outgoing.packet_id = None;

        // Clear retain flag unless retain_as_published
        if !sub_info.retain_as_published {
            outgoing.retain = false;
        }

        // Add ALL subscription identifiers
        for id in sub_info.subscription_ids {
            outgoing.properties.subscription_identifiers.push(id);
        }

        if let Some(sender) = connections.get(&client_id) {
            let _ = sender.try_send(Packet::Publish(outgoing));
        } else {
            // Client disconnected, queue message if persistent session
            if let Some(session) = sessions.get(client_id.as_ref()) {
                let mut s = session.write();
                if !s.clean_start && s.queue_message(outgoing) == QueueResult::DroppedOldest {
                    let _ = events.send(BrokerEvent::MessageDropped);
                }
            }
        }
    }

    // Notify event subscribers (for bridge forwarding and monitoring)
    let _ = events.send(BrokerEvent::MessagePublished {
        topic: publish.topic.clone(),
        payload: publish.payload.clone(),
        qos: publish.qos,
        retain: publish.retain,
    });

    Ok(())
}

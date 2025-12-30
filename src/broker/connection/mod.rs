//! MQTT Connection Handler
//!
//! Handles individual client connections, packet processing,
//! and protocol state machine.
//!
//! Performance optimizations:
//! - Uses AHashMap for faster deduplication during message routing
//! - Uses SmallVec for subscription IDs (typically few per message)
//! - Pre-allocates collections with reasonable capacity

mod connect;
mod disconnect;
mod publish;
mod qos;
mod subscribe;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use dashmap::DashMap;
use parking_lot::RwLock;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::broadcast;
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

use crate::broker::{BrokerConfig, BrokerEvent, RetainedMessage, SharedWriter};
use crate::buffer_pool;
use crate::codec::{Decoder, Encoder};
use crate::hooks::Hooks;
use crate::metrics::Metrics;
use crate::protocol::Packet;
use crate::proxy::ProxyInfo;
use crate::session::{Session, SessionStore};
use crate::topic::SubscriptionStore;

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
pub(crate) enum State {
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
    pub(crate) stream: S,
    pub(crate) addr: SocketAddr,
    pub(crate) state: State,
    pub(crate) decoder: Decoder,
    pub(crate) encoder: Encoder,
    pub(crate) read_buf: BytesMut,
    pub(crate) write_buf: BytesMut,
    pub(crate) sessions: Arc<SessionStore>,
    pub(crate) subscriptions: Arc<SubscriptionStore>,
    pub(crate) retained: Arc<DashMap<String, RetainedMessage>>,
    /// Active connections - maps client_id to SharedWriter for direct writes
    pub(crate) connections: Arc<DashMap<Arc<str>, Arc<SharedWriter>>>,
    pub(crate) config: BrokerConfig,
    pub(crate) events: broadcast::Sender<BrokerEvent>,
    /// SharedWriter for this connection (created after CONNECT)
    pub(crate) shared_writer: Option<Arc<SharedWriter>>,
    pub(crate) hooks: Arc<dyn Hooks>,
    pub(crate) metrics: Option<Arc<Metrics>>,
    /// Persistence manager for durable storage
    pub(crate) persistence: Option<Arc<crate::persistence::PersistenceManager>>,
    /// Username from CONNECT packet (for ACL checks)
    pub(crate) username: Option<String>,
    /// PROXY protocol info (if connection came through a proxy)
    #[allow(dead_code)]
    pub(crate) proxy_info: Option<ProxyInfo>,
}

impl<S> Connection<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        stream: S,
        addr: SocketAddr,
        proxy_info: Option<ProxyInfo>,
        sessions: Arc<SessionStore>,
        subscriptions: Arc<SubscriptionStore>,
        retained: Arc<DashMap<String, RetainedMessage>>,
        connections: Arc<DashMap<Arc<str>, Arc<SharedWriter>>>,
        config: BrokerConfig,
        events: broadcast::Sender<BrokerEvent>,
        hooks: Arc<dyn Hooks>,
        metrics: Option<Arc<Metrics>>,
        persistence: Option<Arc<crate::persistence::PersistenceManager>>,
    ) -> Self {
        Self {
            stream,
            addr,
            state: State::Connecting,
            decoder: Decoder::new().with_max_packet_size(config.max_packet_size),
            encoder: Encoder::default(),
            read_buf: buffer_pool::get_buffer(),
            write_buf: buffer_pool::get_buffer(),
            sessions,
            subscriptions,
            retained,
            connections,
            config,
            events,
            shared_writer: None,
            hooks,
            metrics,
            persistence,
            username: None,
            proxy_info,
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

        let keep_alive_secs = {
            let s = session.read();
            s.keep_alive
        };
        // Calculate 1.5x keep_alive timeout (0 means disabled)
        let keep_alive = if keep_alive_secs > 0 {
            Duration::from_millis(keep_alive_secs as u64 * 1500)
        } else {
            Duration::from_secs(u64::MAX) // Effectively disabled
        };
        debug!(
            "Keep alive for {}: {}s -> timeout {:?}",
            client_id, keep_alive_secs, keep_alive
        );

        // Create retry ticker for unacked QoS 1/2 messages
        let retry_interval = self.config.retry_interval;
        let mut retry_ticker = tokio::time::interval(retry_interval);
        // Skip the first immediate tick
        retry_ticker.tick().await;

        // Track keep-alive deadline (reset when packets received)
        let mut keep_alive_deadline = tokio::time::Instant::now() + keep_alive;

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
                                // Capture raw bytes for PUBLISH (zero-copy fan-out)
                                let raw_bytes = if matches!(&packet, Packet::Publish(_)) {
                                    Some(self.read_buf.split_to(consumed).freeze())
                                } else {
                                    self.read_buf.advance(consumed);
                                    None
                                };

                                // Update activity timestamp and reset keep-alive deadline
                                {
                                    let mut s = session.write();
                                    s.touch();
                                }
                                keep_alive_deadline = tokio::time::Instant::now() + keep_alive;

                                if let Err(e) = self.handle_packet(&client_id, &session, packet, raw_bytes).await {
                                    match &e {
                                        ConnectionError::Shutdown => {
                                            // Normal disconnect, already handled in handle_packet
                                            return Err(e);
                                        }
                                        ConnectionError::Io(_) => {
                                            // IO errors (broken pipe, etc.) are normal during disconnect
                                            debug!("Connection error: {}", e);
                                            self.handle_disconnect(&client_id, &session, true).await;
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
                            debug!("Read error: {}", e);
                            self.handle_disconnect(&client_id, &session, true).await;
                            return Err(e.into());
                        }
                    }
                }

                // Flush outgoing messages from SharedWriter buffer
                _ = async {
                    if let Some(ref writer) = self.shared_writer {
                        writer.notified().await
                    } else {
                        // No shared writer yet, sleep forever (won't happen after CONNECT)
                        std::future::pending::<()>().await
                    }
                } => {
                    if let Some(ref writer) = self.shared_writer {
                        // Take all pending data and write to socket
                        let data = writer.take_buffer();
                        if !data.is_empty() {
                            let bytes_sent = data.len();
                            self.stream.write_all(&data).await?;
                            if let Some(ref metrics) = self.metrics {
                                metrics.publish_sent(bytes_sent);
                            }
                        }
                        // Check if connection was closed
                        if !writer.is_alive() {
                            debug!("SharedWriter closed, disconnecting {}", client_id);
                            self.handle_disconnect(&client_id, &session, false).await;
                            return Err(ConnectionError::Shutdown);
                        }
                    }
                }

                // Retry unacked messages
                _ = retry_ticker.tick() => {
                    self.retry_unacked_messages(&session).await?;
                }

                // Keep alive timeout
                _ = tokio::time::sleep_until(keep_alive_deadline) => {
                    info!("Keep alive timeout for {} - disconnecting", client_id);
                    // For MQTT v5, send DISCONNECT with KeepAliveTimeout reason before closing
                    if self.decoder.protocol_version() == Some(crate::protocol::ProtocolVersion::V5) {
                        let disconnect = crate::protocol::Disconnect {
                            reason_code: crate::protocol::ReasonCode::KeepAliveTimeout,
                            properties: crate::protocol::Properties::default(),
                        };
                        self.write_buf.clear();
                        if self.encoder.encode(&Packet::Disconnect(disconnect), &mut self.write_buf).is_ok() {
                            let _ = self.stream.write_all(&self.write_buf).await;
                            let _ = self.stream.flush().await;
                        }
                    }
                    self.handle_disconnect(&client_id, &session, true).await;
                    return Err(ConnectionError::Timeout);
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
        raw_bytes: Option<bytes::Bytes>,
    ) -> Result<(), ConnectionError> {
        match packet {
            Packet::Connect(_) => {
                // Protocol violation - CONNECT already received
                Err(ConnectionError::Protocol(
                    crate::protocol::ProtocolError::ProtocolViolation("duplicate CONNECT"),
                ))
            }
            Packet::Publish(publish) => self.handle_publish(client_id, session, publish, raw_bytes).await,
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

    /// Return buffers to the pool for reuse by other connections
    pub fn return_buffers(&mut self) {
        let read_buf = std::mem::take(&mut self.read_buf);
        let write_buf = std::mem::take(&mut self.write_buf);
        buffer_pool::put_buffer(read_buf);
        buffer_pool::put_buffer(write_buf);
    }
}

/// Generate a random ID
pub(crate) fn rand_id() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    let hasher = RandomState::new().build_hasher();
    hasher.finish()
}

// Extension trait for BytesMut to add advance method
pub(crate) trait BytesMutExt {
    fn advance(&mut self, cnt: usize);
}

impl BytesMutExt for BytesMut {
    fn advance(&mut self, cnt: usize) {
        bytes::Buf::advance(self, cnt);
    }
}

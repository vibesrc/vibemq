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
use std::time::{Duration, Instant};

use bytes::BytesMut;
use dashmap::DashMap;
use parking_lot::RwLock;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{broadcast, mpsc};
use tokio::time::timeout;
use tracing::{debug, error, warn};

use crate::broker::{BrokerConfig, BrokerEvent, RetainedMessage};
use crate::codec::{Decoder, Encoder};
use crate::hooks::Hooks;
use crate::metrics::Metrics;
use crate::protocol::Packet;
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
    pub(crate) connections: Arc<DashMap<Arc<str>, mpsc::Sender<Packet>>>,
    pub(crate) config: BrokerConfig,
    pub(crate) events: broadcast::Sender<BrokerEvent>,
    pub(crate) packet_tx: mpsc::Sender<Packet>,
    pub(crate) packet_rx: mpsc::Receiver<Packet>,
    pub(crate) hooks: Arc<dyn Hooks>,
    pub(crate) metrics: Option<Arc<Metrics>>,
    /// Username from CONNECT packet (for ACL checks)
    pub(crate) username: Option<String>,
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

        // Create retry ticker for unacked QoS 1/2 messages
        let retry_interval = self.config.retry_interval;
        let mut retry_ticker = tokio::time::interval(retry_interval);
        // Skip the first immediate tick
        retry_ticker.tick().await;

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
                            debug!("Read error: {}", e);
                            self.handle_disconnect(&client_id, &session, true).await;
                            return Err(e.into());
                        }
                    }
                }

                // Receive packets to send
                Some(packet) = self.packet_rx.recv() => {
                    self.handle_outgoing_packet(&session, packet).await?;
                }

                // Retry unacked messages
                _ = retry_ticker.tick() => {
                    self.retry_unacked_messages(&session).await?;
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

    /// Handle outgoing packet from the channel
    async fn handle_outgoing_packet(
        &mut self,
        session: &Arc<RwLock<Session>>,
        packet: Packet,
    ) -> Result<(), ConnectionError> {
        use crate::protocol::QoS;
        use crate::session::{InflightMessage, Qos2State, QueueResult};

        match packet {
            Packet::Disconnect(_) => {
                // We're being disconnected (session takeover)
                // Per MQTT spec, after sending DISCONNECT, we must close the connection
                self.write_buf.clear();
                let _ = self.encoder.encode(&packet, &mut self.write_buf);
                let _ = self.stream.write_all(&self.write_buf).await;
                // Return Shutdown to terminate the connection loop
                Err(ConnectionError::Shutdown)
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
                        debug!("Send quota exhausted for {}, queuing message", s.client_id);
                        if s.queue_message(publish) == QueueResult::DroppedOldest {
                            let _ = self.events.send(BrokerEvent::MessageDropped);
                        }
                        return Ok(());
                    }
                    // Check max_inflight limit
                    if s.inflight_outgoing.len() >= s.max_inflight as usize {
                        // Inflight limit reached - queue and restore quota
                        s.increment_send_quota();
                        debug!(
                            "Inflight limit ({}) reached for {}, queuing message",
                            s.max_inflight, s.client_id
                        );
                        if s.queue_message(publish) == QueueResult::DroppedOldest {
                            let _ = self.events.send(BrokerEvent::MessageDropped);
                        }
                        return Ok(());
                    }
                    // Assign packet ID
                    if publish.packet_id.is_none() {
                        publish.packet_id = Some(s.next_packet_id());
                    }
                    // Store inflight
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
                        "Dropping PUBLISH: encoded size {} exceeds client max {}",
                        self.write_buf.len(),
                        max_packet_size
                    );
                    return Ok(());
                }

                let bytes_sent = self.write_buf.len();
                self.stream.write_all(&self.write_buf).await?;
                if let Some(ref metrics) = self.metrics {
                    metrics.publish_sent(bytes_sent);
                }
                Ok(())
            }
            _ => {
                self.write_buf.clear();
                self.encoder
                    .encode(&packet, &mut self.write_buf)
                    .map_err(|e| ConnectionError::Protocol(e.into()))?;
                self.stream.write_all(&self.write_buf).await?;
                Ok(())
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

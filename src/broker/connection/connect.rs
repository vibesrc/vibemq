//! CONNECT packet handling

use std::sync::Arc;
use std::time::Instant;

use parking_lot::RwLock;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tracing::{debug, error, trace};

use super::{BytesMutExt, Connection, ConnectionError, State};
use crate::broker::BrokerEvent;
use crate::protocol::{
    ConnAck, Disconnect, Packet, Properties, ProtocolVersion, PubRel, QoS, ReasonCode,
};
use crate::session::{
    InflightMessage, Qos2State, QueueResult, Session, SessionLimits, WillMessage,
};

impl<S> Connection<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Read and process CONNECT packet
    pub(crate) async fn read_connect(&mut self) -> Result<(), ConnectionError> {
        loop {
            // Try to decode a packet from the buffer
            match self.decoder.decode(&self.read_buf) {
                Ok(Some((packet, consumed))) => {
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
                Ok(None) => {
                    // Need more data
                }
                Err(e) => {
                    // For MQTT v5, send CONNACK with error before closing
                    if self.decoder.protocol_version() == Some(ProtocolVersion::V5) {
                        let reason_code = match &e {
                            crate::protocol::DecodeError::InvalidProtocolVersion(_) => {
                                ReasonCode::UnsupportedProtocolVersion
                            }
                            crate::protocol::DecodeError::MalformedPacket(_) => {
                                ReasonCode::MalformedPacket
                            }
                            _ => ReasonCode::MalformedPacket,
                        };
                        self.encoder.set_protocol_version(ProtocolVersion::V5);
                        let connack = ConnAck {
                            session_present: false,
                            reason_code,
                            properties: Properties::default(),
                        };
                        let mut buf = bytes::BytesMut::new();
                        if self
                            .encoder
                            .encode(&Packet::ConnAck(connack), &mut buf)
                            .is_ok()
                        {
                            let _ = self.stream.write_all(&buf).await;
                            let _ = self.stream.flush().await;
                        }
                    }
                    return Err(e.into());
                }
            }

            // Read more data
            use tokio::io::AsyncReadExt;
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
    async fn handle_connect(
        &mut self,
        connect: crate::protocol::Connect,
    ) -> Result<(), ConnectionError> {
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
            format!("roker-{:x}", super::rand_id()).into()
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

        // Check max_connections limit
        // Only count as new connection if client_id is not already connected
        let is_takeover = self.connections.contains_key(&client_id);
        if !is_takeover && self.connections.len() >= self.config.max_connections {
            debug!(
                "Max connections ({}) reached, rejecting {}",
                self.config.max_connections, client_id
            );
            let connack = ConnAck {
                session_present: false,
                reason_code: ReasonCode::ServerUnavailable,
                properties: Properties::default(),
            };
            self.write_buf.clear();
            self.encoder
                .encode(&Packet::ConnAck(connack), &mut self.write_buf)
                .map_err(|e| ConnectionError::Protocol(e.into()))?;
            self.stream.write_all(&self.write_buf).await?;
            return Err(ConnectionError::Protocol(
                crate::protocol::ProtocolError::ProtocolViolation("max connections reached"),
            ));
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
        let session_limits = SessionLimits {
            max_pending_messages: self.config.max_queued_messages,
            max_inflight: self.config.max_inflight,
            max_awaiting_rel: self.config.max_awaiting_rel,
        };
        let (session, session_present) = self.sessions.get_or_create(
            &client_id,
            protocol_version,
            connect.clean_start,
            session_limits,
        );

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

        // Send pending messages
        self.send_pending_messages(&session).await?;

        // Re-send unacknowledged inflight messages on session resume [MQTT-4.4.0-1]
        if session_present {
            self.resend_inflight_messages(&session).await?;

            // Send retained messages for existing subscriptions
            self.send_retained_for_existing_subscriptions(&client_id, &session)
                .await?;
        }

        Ok(())
    }

    /// Send pending messages from session queue
    async fn send_pending_messages(
        &mut self,
        session: &Arc<RwLock<Session>>,
    ) -> Result<(), ConnectionError> {
        let (pending, max_packet_size) = {
            let mut s = session.write();
            (s.drain_pending_messages(), s.max_packet_size)
        };

        for mut publish in pending {
            if publish.qos != QoS::AtMostOnce {
                let mut s = session.write();
                // Check send quota (MQTT v5.0 flow control)
                if !s.decrement_send_quota() {
                    // Quota exhausted - re-queue remaining messages
                    if s.queue_message(publish) == QueueResult::DroppedOldest {
                        let _ = self.events.send(BrokerEvent::MessageDropped);
                    }
                    continue;
                }
                // Check max_inflight limit
                if s.inflight_outgoing.len() >= s.max_inflight as usize {
                    // Inflight limit reached - re-queue and restore quota
                    s.increment_send_quota();
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
                tracing::warn!(
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

        Ok(())
    }

    /// Re-send unacknowledged inflight messages on session resume
    ///
    /// Per [MQTT-4.4.0-1]: When a Client reconnects with CleanSession set to 0,
    /// both Client and Server MUST re-send any unacknowledged PUBLISH packets
    /// (where QoS > 0) and PUBREL packets using their original Packet Identifiers.
    async fn resend_inflight_messages(
        &mut self,
        session: &Arc<RwLock<Session>>,
    ) -> Result<(), ConnectionError> {
        let (to_resend, max_packet_size) = {
            let mut s = session.write();
            let now = Instant::now();
            let messages: Vec<_> = s
                .inflight_outgoing
                .iter_mut()
                .map(|(packet_id, inflight)| {
                    // Update sent_at for retry tracking
                    inflight.sent_at = now;
                    inflight.retry_count += 1;
                    (*packet_id, inflight.publish.clone(), inflight.qos2_state)
                })
                .collect();
            (messages, s.max_packet_size)
        };

        for (packet_id, mut publish, qos2_state) in to_resend {
            match qos2_state {
                None | Some(Qos2State::WaitingPubRec) => {
                    // QoS 1, or QoS 2 waiting for PUBREC: resend PUBLISH with DUP=1 [MQTT-3.3.1-1]
                    publish.dup = true;
                    publish.packet_id = Some(packet_id);

                    self.write_buf.clear();
                    self.encoder
                        .encode(&Packet::Publish(publish), &mut self.write_buf)
                        .map_err(|e| ConnectionError::Protocol(e.into()))?;

                    if self.write_buf.len() <= max_packet_size as usize {
                        trace!(
                            "Resending inflight PUBLISH packet_id={} with DUP=1",
                            packet_id
                        );
                        self.stream.write_all(&self.write_buf).await?;
                    }
                }
                Some(Qos2State::WaitingPubComp) => {
                    // QoS 2 waiting for PUBCOMP: resend PUBREL
                    let pubrel = PubRel::new(packet_id);
                    self.write_buf.clear();
                    self.encoder
                        .encode(&Packet::PubRel(pubrel), &mut self.write_buf)
                        .map_err(|e| ConnectionError::Protocol(e.into()))?;

                    trace!("Resending inflight PUBREL packet_id={}", packet_id);
                    self.stream.write_all(&self.write_buf).await?;
                }
            }
        }

        Ok(())
    }

    /// Send retained messages for existing subscriptions on session resume
    async fn send_retained_for_existing_subscriptions(
        &mut self,
        client_id: &Arc<str>,
        session: &Arc<RwLock<Session>>,
    ) -> Result<(), ConnectionError> {
        let subs: Vec<_> = {
            let s = session.read();
            s.subscriptions.values().cloned().collect()
        };

        for sub in subs {
            self.send_retained_messages(
                client_id,
                &sub.filter,
                sub.options.qos,
                session,
                sub.subscription_id,
            )
            .await?;
        }

        Ok(())
    }
}

//! PUBLISH packet handling and message routing

use std::sync::Arc;
use std::time::Instant;

use ahash::AHashMap;
use parking_lot::RwLock;
use smallvec::SmallVec;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tracing::{debug, error, trace, warn};

use super::{Connection, ConnectionError};
use crate::broker::{BrokerEvent, RetainedMessage};
use crate::protocol::{Packet, Properties, PubAck, PubRec, Publish, QoS, ReasonCode};
use crate::session::{QueueResult, Session};
use crate::topic::validate_topic_name_with_max_levels;

impl<S> Connection<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Handle PUBLISH packet
    pub(crate) async fn handle_publish(
        &mut self,
        client_id: &Arc<str>,
        session: &Arc<RwLock<Session>>,
        mut publish: Publish,
    ) -> Result<(), ConnectionError> {
        // Validate topic name
        if let Err(e) =
            validate_topic_name_with_max_levels(&publish.topic, self.config.max_topic_levels)
        {
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

                // Check max_awaiting_rel limit
                let limit_exceeded = {
                    let s = session.read();
                    s.inflight_incoming.len() >= s.max_awaiting_rel
                };

                if limit_exceeded {
                    // Send PUBREC with QuotaExceeded - client should retry later
                    debug!("Max awaiting PUBREL limit reached, rejecting QoS 2 publish");
                    let pubrec = PubRec {
                        packet_id,
                        reason_code: ReasonCode::QuotaExceeded,
                        properties: Properties::default(),
                    };
                    self.write_buf.clear();
                    self.encoder
                        .encode(&Packet::PubRec(pubrec), &mut self.write_buf)
                        .map_err(|e| ConnectionError::Protocol(e.into()))?;
                    self.stream.write_all(&self.write_buf).await?;
                    return Ok(());
                }

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
    pub(crate) async fn route_message(
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
                if let Err(tokio::sync::mpsc::error::TrySendError::Full(_)) =
                    sender.try_send(Packet::Publish(outgoing))
                {
                    warn!(client_id = %client_id, "channel full - dropping message");
                }
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
}

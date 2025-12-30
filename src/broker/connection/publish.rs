//! PUBLISH packet handling and message routing

use std::cell::RefCell;
use std::sync::Arc;
use std::time::Instant;

use ahash::AHashMap;
use parking_lot::RwLock;
use smallvec::SmallVec;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tracing::{debug, error, trace, warn};

// Thread-local dedup map for route_message to avoid per-publish allocation.
// Key: client_id, Value: aggregated subscription info
// Capacity 256 reduces reallocations for moderate fan-outs
thread_local! {
    static DEDUP_MAP: RefCell<AHashMap<Arc<str>, ClientSub>> =
        RefCell::new(AHashMap::with_capacity(256));
}

/// Aggregated subscription info for a single client during message routing
struct ClientSub {
    qos: QoS,
    retain_as_published: bool,
    subscription_ids: SmallVec<[u32; 4]>,
}

use super::{Connection, ConnectionError};
use crate::broker::{BrokerEvent, RetainedMessage};
use crate::codec::{PublishCache, RawPublish};
use crate::persistence::{PersistenceOp, StoredRetainedMessage};
use crate::protocol::{Packet, Properties, ProtocolVersion, PubAck, PubRec, Publish, QoS, ReasonCode};
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
        mut raw_bytes: Option<bytes::Bytes>,
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
                // Lookup alias - raw bytes no longer valid (topic resolved from alias)
                raw_bytes = None;
                let s = session.read();
                if let Some(topic) = s.resolve_topic_alias(alias) {
                    publish.topic = Arc::from(topic.as_str());
                } else {
                    // Invalid alias
                    return Err(ConnectionError::Protocol(
                        crate::protocol::ProtocolError::ProtocolViolation("unknown topic alias"),
                    ));
                }
            } else {
                // Register alias
                let mut s = session.write();
                s.register_topic_alias(alias, publish.topic.to_string());
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
                    let topic_str = publish.topic.to_string();
                    if publish.payload.is_empty() {
                        self.retained.remove(&topic_str);
                        if let Some(ref persistence) = self.persistence {
                            persistence.write(PersistenceOp::DeleteRetained { topic: topic_str });
                        }
                    } else {
                        let retained_msg = RetainedMessage {
                            topic: publish.topic.clone(),
                            payload: publish.payload.clone(),
                            qos: publish.qos,
                            properties: publish.properties.clone(),
                            timestamp: Instant::now(),
                        };
                        self.retained
                            .insert(topic_str.clone(), retained_msg.clone());
                        if let Some(ref persistence) = self.persistence {
                            persistence.write(PersistenceOp::SetRetained {
                                topic: topic_str,
                                message: StoredRetainedMessage::from(&retained_msg),
                            });
                        }
                    }
                }
                return Ok(());
            }
        }

        // Handle retained message
        if publish.retain && self.config.retain_available {
            let topic_str = publish.topic.to_string();
            if publish.payload.is_empty() {
                self.retained.remove(&topic_str);
                if let Some(ref persistence) = self.persistence {
                    persistence.write(PersistenceOp::DeleteRetained { topic: topic_str });
                }
            } else {
                let retained_msg = RetainedMessage {
                    topic: publish.topic.clone(),
                    payload: publish.payload.clone(),
                    qos: publish.qos,
                    properties: publish.properties.clone(),
                    timestamp: Instant::now(),
                };
                self.retained
                    .insert(topic_str.clone(), retained_msg.clone());
                if let Some(ref persistence) = self.persistence {
                    persistence.write(PersistenceOp::SetRetained {
                        topic: topic_str,
                        message: StoredRetainedMessage::from(&retained_msg),
                    });
                }
            }
        }

        // Route message to subscribers
        self.route_message(client_id, &publish, raw_bytes).await?;

        Ok(())
    }

    /// Route a message to subscribers
    /// Uses thread-local AHashMap for O(n) deduplication, avoiding per-publish allocation.
    /// Uses RawPublish for zero-copy fan-out when raw_bytes available, falls back to CachedPublish.
    pub(crate) async fn route_message(
        &self,
        sender_id: &Arc<str>,
        publish: &Publish,
        raw_bytes: Option<bytes::Bytes>,
    ) -> Result<(), ConnectionError> {
        let matches = self.subscriptions.matches(&publish.topic);

        // Create RawPublish for zero-copy fan-out if we have raw bytes
        let raw_publish: Option<std::sync::Arc<RawPublish>> = raw_bytes.and_then(|bytes| {
            let version = self.decoder.protocol_version().unwrap_or(ProtocolVersion::V311);
            RawPublish::from_wire(bytes, version).ok().map(std::sync::Arc::new)
        });

        // Fallback cache for re-encoding (used when raw bytes not available or protocol mismatch)
        let mut publish_cache: Option<PublishCache> = None;

        // Use thread-local map for deduplication (reused across publishes)
        DEDUP_MAP.with(|map_cell| {
            let mut client_subs = map_cell.borrow_mut();
            client_subs.clear(); // Reuse allocation from previous calls

            // Deduplicate by client_id, keeping highest QoS and collecting ALL subscription IDs
            for sub in &matches {
                // Skip sender if no_local is set
                if sub.no_local && sub.client_id == *sender_id {
                    continue;
                }

                if let Some(entry) = client_subs.get_mut(&sub.client_id) {
                    // Update existing entry
                    if sub.qos > entry.qos {
                        entry.qos = sub.qos;
                    }
                    if sub.retain_as_published {
                        entry.retain_as_published = true;
                    }
                    if let Some(id) = sub.subscription_id {
                        if !entry.subscription_ids.contains(&id) {
                            entry.subscription_ids.push(id);
                        }
                    }
                } else {
                    // New client - add entry
                    let mut subscription_ids = SmallVec::new();
                    if let Some(id) = sub.subscription_id {
                        subscription_ids.push(id);
                    }
                    client_subs.insert(
                        sub.client_id.clone(),
                        ClientSub {
                            qos: sub.qos,
                            retain_as_published: sub.retain_as_published,
                            subscription_ids,
                        },
                    );
                }
            }

            // Initialize cache if we have subscribers (lazy init)
            if !client_subs.is_empty() {
                publish_cache = Some(PublishCache::new());
            }

            // Send to each client - drain to take ownership without reallocating
            for (client_id, sub_info) in client_subs.drain() {
                let effective_qos = publish.qos.min(sub_info.qos);
                let effective_retain = if sub_info.retain_as_published {
                    publish.retain
                } else {
                    false
                };

                // Use CachedPublish when no subscription identifiers
                // (subscription IDs are encoded in properties and can't be patched)
                let can_use_cached = sub_info.subscription_ids.is_empty();

                if let Some(writer) = self.connections.get(&client_id) {
                    if can_use_cached {
                        // Fast path: pre-serialized bytes with direct write
                        // Get protocol version directly from SharedWriter (no lock needed)
                        let version = writer.protocol_version();

                        // Try zero-copy path first (RawPublish from incoming wire bytes)
                        if let Some(ref raw) = raw_publish {
                            // Use RawPublish if protocol version matches and QoS is supported
                            if raw.protocol_version() == version && raw.supports_qos(effective_qos) {
                                if let Err(e) = writer.send_raw(raw, effective_qos, effective_retain) {
                                    trace!(client_id = %client_id, error = ?e, "send_raw failed");
                                }
                                continue;
                            }
                        }

                        // Fallback to CachedPublish (re-encode)
                        if publish_cache.is_none() {
                            publish_cache = Some(PublishCache::new());
                        }
                        if let Some(ref mut cache) = publish_cache {
                            // Use appropriate cache based on QoS
                            // get_or_create returns Arc clone (atomic increment, no allocation)
                            let cached_result = if effective_qos == QoS::AtMostOnce {
                                cache.get_or_create_qos0(publish, version)
                            } else {
                                cache.get_or_create(publish, version)
                            };

                            if let Ok(cached_arc) = cached_result {
                                // Direct write to shared buffer (no channel overhead)
                                if let Err(e) = writer.send_cached(&cached_arc, effective_qos, effective_retain) {
                                    trace!(client_id = %client_id, error = ?e, "send_cached failed");
                                }
                                continue;
                            }
                        }
                    }

                    // Fallback path: clone and modify Publish (subscription IDs present)
                    let mut outgoing = publish.clone();

                    // Add ALL subscription identifiers
                    for id in &sub_info.subscription_ids {
                        outgoing.properties.subscription_identifiers.push(*id);
                    }

                    // Direct write with full publish encoding
                    if let Err(e) = writer.send_publish(&mut outgoing, effective_qos, effective_retain) {
                        trace!(client_id = %client_id, error = ?e, "send_publish failed");
                    }
                } else {
                    // Client disconnected, queue message if persistent session
                    if let Some(session) = self.sessions.get(client_id.as_ref()) {
                        let mut s = session.write();
                        if !s.clean_start {
                            let mut outgoing = publish.clone();
                            outgoing.qos = effective_qos;
                            outgoing.dup = false;
                            outgoing.packet_id = None;
                            outgoing.retain = effective_retain;

                            for id in &sub_info.subscription_ids {
                                outgoing.properties.subscription_identifiers.push(*id);
                            }

                            if s.queue_message(outgoing) == QueueResult::DroppedOldest {
                                let _ = self.events.send(BrokerEvent::MessageDropped);
                            }
                        }
                    }
                }
            }
        });

        // Notify event subscribers (for bridge forwarding and monitoring)
        let _ = self.events.send(BrokerEvent::MessagePublished {
            topic: publish.topic.to_string(),
            payload: publish.payload.clone(),
            qos: publish.qos,
            retain: publish.retain,
        });

        Ok(())
    }
}

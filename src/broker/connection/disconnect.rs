//! Disconnect handling and will message publishing

use std::sync::Arc;
use std::time::{Duration, Instant};

use ahash::AHashMap;
use dashmap::DashMap;
use parking_lot::RwLock;
use smallvec::SmallVec;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::broadcast;
use tracing::debug;

use super::{Connection, ConnectionError};
use crate::broker::{BrokerEvent, RetainedMessage, SharedWriter};
use crate::persistence::{PersistenceOp, StoredRetainedMessage, StoredSession};
use crate::protocol::{Publish, QoS};
use crate::session::{QueueResult, Session, SessionStore};
use crate::topic::SubscriptionStore;

impl<S> Connection<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Handle disconnection
    pub(crate) async fn handle_disconnect(
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
                    let persistence = self.persistence.clone();
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

                                let topic_arc: Arc<str> = Arc::from(will.topic.as_str());
                                let publish = Publish {
                                    dup: false,
                                    qos: will.qos,
                                    retain: will.retain,
                                    topic: topic_arc.clone(),
                                    packet_id: None,
                                    payload: will.payload,
                                    properties: will.properties,
                                };

                                // Handle retained
                                if will.retain && config.retain_available {
                                    if publish.payload.is_empty() {
                                        retained.remove(&will.topic);
                                        if let Some(ref persistence) = persistence {
                                            persistence.write(PersistenceOp::DeleteRetained {
                                                topic: will.topic.clone(),
                                            });
                                        }
                                    } else {
                                        let retained_msg = RetainedMessage {
                                            topic: topic_arc.clone(),
                                            payload: publish.payload.clone(),
                                            qos: publish.qos,
                                            properties: publish.properties.clone(),
                                            timestamp: Instant::now(),
                                        };
                                        retained.insert(will.topic.clone(), retained_msg.clone());
                                        if let Some(ref persistence) = persistence {
                                            persistence.write(PersistenceOp::SetRetained {
                                                topic: will.topic.clone(),
                                                message: StoredRetainedMessage::from(&retained_msg),
                                            });
                                        }
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
                    let topic_arc: Arc<str> = Arc::from(will.topic.as_str());
                    let publish = Publish {
                        dup: false,
                        qos: will.qos,
                        retain: will.retain,
                        topic: topic_arc.clone(),
                        packet_id: None,
                        payload: will.payload,
                        properties: will.properties,
                    };

                    // Handle retained
                    if will.retain && self.config.retain_available {
                        if publish.payload.is_empty() {
                            self.retained.remove(&will.topic);
                            if let Some(ref persistence) = self.persistence {
                                persistence.write(PersistenceOp::DeleteRetained {
                                    topic: will.topic.clone(),
                                });
                            }
                        } else {
                            let retained_msg = RetainedMessage {
                                topic: topic_arc.clone(),
                                payload: publish.payload.clone(),
                                qos: publish.qos,
                                properties: publish.properties.clone(),
                                timestamp: Instant::now(),
                            };
                            self.retained
                                .insert(will.topic.clone(), retained_msg.clone());
                            if let Some(ref persistence) = self.persistence {
                                persistence.write(PersistenceOp::SetRetained {
                                    topic: will.topic.clone(),
                                    message: StoredRetainedMessage::from(&retained_msg),
                                });
                            }
                        }
                    }

                    // Route will message (no raw bytes - will message is constructed)
                    let _ = self.route_message(client_id, &publish, None).await;

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

        // Persist session on disconnect if non-ephemeral
        if let Some(ref persistence) = self.persistence {
            let s = session.read();
            // Only persist non-clean sessions with expiry > 0
            if !s.clean_start && s.session_expiry_interval > 0 {
                persistence.write(PersistenceOp::SetSession {
                    client_id: client_id.to_string(),
                    session: StoredSession::from_session(&s),
                });
            } else if s.clean_start || s.session_expiry_interval == 0 {
                // Delete any persisted session for clean start or expired
                persistence.write(PersistenceOp::DeleteSession {
                    client_id: client_id.to_string(),
                });
            }
        }

        // Notify event subscribers
        let _ = self.events.send(BrokerEvent::ClientDisconnected {
            client_id: client_id.clone(),
        });

        debug!("Client {} disconnected", client_id);
    }
}

/// Route a will message to subscribers (standalone function for delayed will tasks)
/// Performance: Uses AHashMap for deduplication and SmallVec for subscription IDs
pub(crate) async fn route_will_message(
    subscriptions: &SubscriptionStore,
    connections: &DashMap<Arc<str>, Arc<SharedWriter>>,
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

        if let Some(writer) = connections.get(&client_id) {
            let effective_retain = if sub_info.retain_as_published {
                outgoing.retain
            } else {
                false
            };
            let _ = writer.send_publish(&mut outgoing, effective_qos, effective_retain);
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
        topic: publish.topic.to_string(),
        payload: publish.payload.clone(),
        qos: publish.qos,
        retain: publish.retain,
    });

    Ok(())
}

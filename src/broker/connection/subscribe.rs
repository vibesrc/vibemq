//! SUBSCRIBE and UNSUBSCRIBE packet handling

use std::sync::Arc;

use parking_lot::RwLock;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tracing::{debug, error};

use super::{Connection, ConnectionError};
use crate::broker::BrokerEvent;
use crate::protocol::{
    Packet, Properties, ProtocolVersion, Publish, QoS, ReasonCode, RetainHandling, SubAck,
    Subscribe, UnsubAck, Unsubscribe,
};
use crate::session::Session;
use crate::topic::{validate_topic_filter, Subscription};

impl<S> Connection<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Handle SUBSCRIBE packet
    pub(crate) async fn handle_subscribe(
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
    pub(crate) async fn send_retained_messages(
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
    pub(crate) async fn handle_unsubscribe(
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
}

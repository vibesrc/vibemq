//! QoS acknowledgment handling (PUBACK, PUBREC, PUBREL, PUBCOMP)

use std::sync::Arc;
use std::time::Instant;

use parking_lot::RwLock;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tracing::trace;

use super::{Connection, ConnectionError};
use crate::protocol::{Packet, PubAck, PubComp, PubRec, PubRel};
use crate::session::{Qos2State, Session};

impl<S> Connection<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Handle PUBACK packet
    pub(crate) async fn handle_puback(
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
    pub(crate) async fn handle_pubrec(
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
    pub(crate) async fn handle_pubrel(
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
    pub(crate) async fn handle_pubcomp(
        &mut self,
        session: &Arc<RwLock<Session>>,
        pubcomp: PubComp,
    ) -> Result<(), ConnectionError> {
        let mut s = session.write();
        s.inflight_outgoing.remove(&pubcomp.packet_id);
        s.increment_send_quota();
        Ok(())
    }

    /// Retry unacked QoS 1/2 messages
    pub(crate) async fn retry_unacked_messages(
        &mut self,
        session: &Arc<RwLock<Session>>,
    ) -> Result<(), ConnectionError> {
        let now = Instant::now();
        let retry_interval = self.config.retry_interval;

        // Collect messages that need retry (to avoid holding lock while sending)
        let to_retry: Vec<_> = {
            let mut s = session.write();
            s.inflight_outgoing
                .iter_mut()
                .filter_map(|(packet_id, inflight)| {
                    if now.duration_since(inflight.sent_at) >= retry_interval {
                        // Update retry metadata
                        inflight.retry_count += 1;
                        inflight.sent_at = now;

                        Some((*packet_id, inflight.publish.clone(), inflight.qos2_state))
                    } else {
                        None
                    }
                })
                .collect()
        };

        // Get max packet size
        let max_packet_size = {
            let s = session.read();
            s.max_packet_size
        };

        // Send retries
        for (packet_id, mut publish, qos2_state) in to_retry {
            match qos2_state {
                None | Some(Qos2State::WaitingPubRec) => {
                    // QoS 1, or QoS 2 waiting for PUBREC: resend PUBLISH with DUP flag
                    publish.dup = true;
                    publish.packet_id = Some(packet_id);

                    self.write_buf.clear();
                    self.encoder
                        .encode(&Packet::Publish(publish), &mut self.write_buf)
                        .map_err(|e| ConnectionError::Protocol(e.into()))?;

                    if self.write_buf.len() <= max_packet_size as usize {
                        trace!("Retrying PUBLISH packet_id={}", packet_id);
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

                    trace!("Retrying PUBREL packet_id={}", packet_id);
                    self.stream.write_all(&self.write_buf).await?;
                }
            }
        }

        Ok(())
    }
}

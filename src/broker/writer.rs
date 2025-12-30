//! Direct write buffer for bypassing channel overhead.
//!
//! SharedWriter allows the router to write directly to a per-connection buffer,
//! eliminating mpsc channel overhead for message fan-out.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use bytes::BytesMut;
use parking_lot::{Mutex, RwLock};
use tokio::sync::Notify;

use crate::codec::{CachedPublish, Encoder, RawPublish};
use crate::protocol::{Packet, ProtocolVersion, QoS};
use crate::session::{InflightMessage, Qos2State, Session};

/// Error when sending to a SharedWriter
#[derive(Debug)]
pub enum SendError {
    /// Connection is closed
    Closed,
    /// Send quota exceeded (flow control)
    QuotaExceeded,
    /// Inflight limit reached
    InflightLimitExceeded,
    /// Encoding error
    EncodingError,
}

/// Shared write buffer for direct writes from router to connection.
///
/// The router appends pre-serialized bytes to the buffer, and the connection
/// flushes the buffer to the socket. This eliminates channel overhead.
pub struct SharedWriter {
    /// Pre-serialized bytes waiting to be written
    buffer: Mutex<BytesMut>,
    /// Session for packet_id assignment and inflight tracking
    session: Arc<RwLock<Session>>,
    /// Notification when buffer has new data
    notify: Notify,
    /// Protocol version for encoding non-cached packets
    protocol_version: ProtocolVersion,
    /// Encoder for non-cached packets
    encoder: Mutex<Encoder>,
    /// Whether the connection is still alive
    alive: AtomicBool,
    /// Maximum packet size for this client
    max_packet_size: u32,
}

impl SharedWriter {
    /// Create a new SharedWriter
    pub fn new(
        session: Arc<RwLock<Session>>,
        protocol_version: ProtocolVersion,
        max_packet_size: u32,
    ) -> Self {
        Self {
            buffer: Mutex::new(BytesMut::with_capacity(2048)), // 2KB initial, grows as needed
            session,
            notify: Notify::new(),
            protocol_version,
            encoder: Mutex::new(Encoder::new(protocol_version)),
            alive: AtomicBool::new(true),
            max_packet_size,
        }
    }

    /// Check if the connection is still alive
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    /// Get the protocol version for this connection
    pub fn protocol_version(&self) -> ProtocolVersion {
        self.protocol_version
    }

    /// Mark the connection as closed
    pub fn close(&self) {
        self.alive.store(false, Ordering::Release);
        self.notify.notify_one();
    }

    /// Get the notify handle for the connection loop
    pub fn notified(&self) -> tokio::sync::futures::Notified<'_> {
        self.notify.notified()
    }

    /// Take all pending data from the buffer
    pub fn take_buffer(&self) -> BytesMut {
        let mut buf = self.buffer.lock();
        buf.split()
    }

    /// Get buffer length (for metrics/debugging)
    pub fn buffer_len(&self) -> usize {
        self.buffer.lock().len()
    }

    /// Send a pre-serialized CachedPublish to this connection.
    ///
    /// For QoS 0: Just appends bytes to the buffer.
    /// For QoS 1/2: Assigns packet_id, stores in inflight, then appends bytes.
    ///
    /// Returns Ok(()) on success, or an error if flow control limits are exceeded.
    pub fn send_cached(
        &self,
        cached: &Arc<CachedPublish>,
        qos: QoS,
        retain: bool,
    ) -> Result<(), SendError> {
        if !self.is_alive() {
            return Err(SendError::Closed);
        }

        // Assign packet_id for QoS > 0
        let packet_id = if qos != QoS::AtMostOnce {
            let mut session = self.session.write();

            // Flow control: check send quota
            if !session.decrement_send_quota() {
                return Err(SendError::QuotaExceeded);
            }

            // Check inflight limit
            if session.inflight_outgoing.len() >= session.max_inflight as usize {
                session.increment_send_quota();
                return Err(SendError::InflightLimitExceeded);
            }

            let pid = session.next_packet_id();

            // Store in inflight for retransmission
            session.inflight_outgoing.insert(
                pid,
                InflightMessage::Cached {
                    packet_id: pid,
                    cached: cached.clone(),
                    qos,
                    retain,
                    qos2_state: if qos == QoS::ExactlyOnce {
                        Some(Qos2State::WaitingPubRec)
                    } else {
                        None
                    },
                    sent_at: Instant::now(),
                    retry_count: 0,
                },
            );

            Some(pid)
        } else {
            None
        };

        // Write to buffer
        let was_empty = {
            let mut buffer = self.buffer.lock();
            let start_len = buffer.len();
            cached.write_to(&mut buffer, packet_id, qos, retain, false);

            // Check packet size limit
            let packet_len = buffer.len() - start_len;
            if packet_len > self.max_packet_size as usize {
                // Remove the oversized packet
                buffer.truncate(start_len);
                // Undo inflight tracking if we added it
                if let Some(pid) = packet_id {
                    let mut session = self.session.write();
                    session.inflight_outgoing.remove(&pid);
                    session.increment_send_quota();
                }
                return Ok(()); // Silently drop oversized packets
            }
            start_len == 0
        };

        // Only notify if buffer was empty - coalesces notifications during bursts
        if was_empty {
            self.notify.notify_one();
        }

        Ok(())
    }

    /// Send a RawPublish (zero-copy from incoming wire bytes).
    ///
    /// For QoS 0: Just appends bytes to the buffer with patching.
    /// For QoS 1/2: Assigns packet_id, stores in inflight, then appends bytes.
    ///
    /// Returns Ok(()) on success, or an error if flow control limits are exceeded.
    pub fn send_raw(
        &self,
        raw: &Arc<RawPublish>,
        qos: QoS,
        retain: bool,
    ) -> Result<(), SendError> {
        if !self.is_alive() {
            return Err(SendError::Closed);
        }

        // Assign packet_id for QoS > 0
        let packet_id = if qos != QoS::AtMostOnce {
            let mut session = self.session.write();

            // Flow control: check send quota
            if !session.decrement_send_quota() {
                return Err(SendError::QuotaExceeded);
            }

            // Check inflight limit
            if session.inflight_outgoing.len() >= session.max_inflight as usize {
                session.increment_send_quota();
                return Err(SendError::InflightLimitExceeded);
            }

            let pid = session.next_packet_id();

            // Store in inflight for retransmission
            session.inflight_outgoing.insert(
                pid,
                InflightMessage::Raw {
                    packet_id: pid,
                    raw: raw.clone(),
                    qos,
                    retain,
                    qos2_state: if qos == QoS::ExactlyOnce {
                        Some(Qos2State::WaitingPubRec)
                    } else {
                        None
                    },
                    sent_at: Instant::now(),
                    retry_count: 0,
                },
            );

            Some(pid)
        } else {
            None
        };

        // Write to buffer
        let was_empty = {
            let mut buffer = self.buffer.lock();
            let start_len = buffer.len();
            raw.write_to(&mut buffer, packet_id, qos, retain, false);

            // Check packet size limit
            let packet_len = buffer.len() - start_len;
            if packet_len > self.max_packet_size as usize {
                // Remove the oversized packet
                buffer.truncate(start_len);
                // Undo inflight tracking if we added it
                if let Some(pid) = packet_id {
                    let mut session = self.session.write();
                    session.inflight_outgoing.remove(&pid);
                    session.increment_send_quota();
                }
                return Ok(()); // Silently drop oversized packets
            }
            start_len == 0
        };

        // Only notify if buffer was empty - coalesces notifications during bursts
        if was_empty {
            self.notify.notify_one();
        }

        Ok(())
    }

    /// Send a non-PUBLISH packet (PUBACK, SUBACK, etc.)
    pub fn send_packet(&self, packet: &Packet) -> Result<(), SendError> {
        if !self.is_alive() {
            return Err(SendError::Closed);
        }

        let was_empty = {
            let mut buffer = self.buffer.lock();
            let encoder = self.encoder.lock();

            let start_len = buffer.len();
            if encoder.encode(packet, &mut buffer).is_err() {
                buffer.truncate(start_len);
                return Err(SendError::EncodingError);
            }

            // Check packet size limit
            let packet_len = buffer.len() - start_len;
            if packet_len > self.max_packet_size as usize {
                buffer.truncate(start_len);
                return Ok(()); // Silently drop oversized packets
            }
            start_len == 0
        };

        // Only notify if buffer was empty - coalesces notifications during bursts
        if was_empty {
            self.notify.notify_one();
        }
        Ok(())
    }

    /// Send a raw PUBLISH packet (with subscription identifiers).
    /// Used as fallback when CachedPublish can't be used.
    pub fn send_publish(
        &self,
        publish: &mut crate::protocol::Publish,
        effective_qos: QoS,
        effective_retain: bool,
    ) -> Result<(), SendError> {
        if !self.is_alive() {
            return Err(SendError::Closed);
        }

        // Apply effective QoS and retain
        publish.qos = effective_qos;
        publish.retain = effective_retain;
        publish.dup = false;

        // Assign packet_id for QoS > 0
        if effective_qos != QoS::AtMostOnce {
            let mut session = self.session.write();

            // Flow control
            if !session.decrement_send_quota() {
                return Err(SendError::QuotaExceeded);
            }

            if session.inflight_outgoing.len() >= session.max_inflight as usize {
                session.increment_send_quota();
                return Err(SendError::InflightLimitExceeded);
            }

            let pid = session.next_packet_id();
            publish.packet_id = Some(pid);

            // Store in inflight
            session.inflight_outgoing.insert(
                pid,
                InflightMessage::Full {
                    packet_id: pid,
                    publish: publish.clone(),
                    qos2_state: if effective_qos == QoS::ExactlyOnce {
                        Some(Qos2State::WaitingPubRec)
                    } else {
                        None
                    },
                    sent_at: Instant::now(),
                    retry_count: 0,
                },
            );
        } else {
            publish.packet_id = None;
        }

        // Encode to buffer
        let was_empty = {
            let mut buffer = self.buffer.lock();
            let encoder = self.encoder.lock();

            let start_len = buffer.len();
            if encoder
                .encode(&Packet::Publish(publish.clone()), &mut buffer)
                .is_err()
            {
                buffer.truncate(start_len);
                // Undo inflight tracking
                if let Some(pid) = publish.packet_id {
                    drop(encoder);
                    drop(buffer);
                    let mut session = self.session.write();
                    session.inflight_outgoing.remove(&pid);
                    session.increment_send_quota();
                }
                return Err(SendError::EncodingError);
            }

            // Check packet size limit
            let packet_len = buffer.len() - start_len;
            if packet_len > self.max_packet_size as usize {
                buffer.truncate(start_len);
                // Undo inflight tracking
                if let Some(pid) = publish.packet_id {
                    drop(encoder);
                    drop(buffer);
                    let mut session = self.session.write();
                    session.inflight_outgoing.remove(&pid);
                    session.increment_send_quota();
                }
                return Ok(()); // Silently drop
            }
            start_len == 0
        };

        // Only notify if buffer was empty - coalesces notifications during bursts
        if was_empty {
            self.notify.notify_one();
        }
        Ok(())
    }

    /// Get the session reference (for connection to use during reads)
    pub fn session(&self) -> &Arc<RwLock<Session>> {
        &self.session
    }
}

impl std::fmt::Debug for SharedWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedWriter")
            .field("buffer_len", &self.buffer_len())
            .field("alive", &self.is_alive())
            .field("protocol_version", &self.protocol_version)
            .finish()
    }
}

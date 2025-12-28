//! Message Router
//!
//! Handles message routing between publishers and subscribers.

use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::mpsc;

use crate::broker::OutboundMessage;
use crate::protocol::Packet;

/// Message router for distributing messages to subscribers
pub struct MessageRouter {
    /// Client send channels
    clients: Arc<DashMap<Arc<str>, mpsc::Sender<OutboundMessage>>>,
}

impl MessageRouter {
    pub fn new(clients: Arc<DashMap<Arc<str>, mpsc::Sender<OutboundMessage>>>) -> Self {
        Self { clients }
    }

    /// Send a message to a specific client
    pub async fn send_to_client(&self, client_id: &Arc<str>, msg: OutboundMessage) -> bool {
        if let Some(sender) = self.clients.get(client_id) {
            sender.send(msg).await.is_ok()
        } else {
            false
        }
    }

    /// Send a packet to a specific client (convenience wrapper)
    pub async fn send_packet_to_client(&self, client_id: &Arc<str>, packet: Packet) -> bool {
        self.send_to_client(client_id, OutboundMessage::Packet(packet))
            .await
    }

    /// Try to send a message to a specific client (non-blocking)
    pub fn try_send_to_client(&self, client_id: &Arc<str>, msg: OutboundMessage) -> bool {
        if let Some(sender) = self.clients.get(client_id) {
            match sender.try_send(msg) {
                Ok(()) => true,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    tracing::warn!(client_id = %client_id, "channel full - backpressure");
                    false
                }
                Err(mpsc::error::TrySendError::Closed(_)) => false,
            }
        } else {
            false
        }
    }

    /// Try to send a packet to a specific client (convenience wrapper)
    pub fn try_send_packet_to_client(&self, client_id: &Arc<str>, packet: Packet) -> bool {
        self.try_send_to_client(client_id, OutboundMessage::Packet(packet))
    }
}

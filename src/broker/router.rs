//! Message Router
//!
//! Handles message routing between publishers and subscribers.

use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::mpsc;

use crate::protocol::Packet;

/// Message router for distributing messages to subscribers
pub struct MessageRouter {
    /// Client send channels
    clients: Arc<DashMap<Arc<str>, mpsc::Sender<Packet>>>,
}

impl MessageRouter {
    pub fn new(clients: Arc<DashMap<Arc<str>, mpsc::Sender<Packet>>>) -> Self {
        Self { clients }
    }

    /// Send a message to a specific client
    pub async fn send_to_client(&self, client_id: &Arc<str>, packet: Packet) -> bool {
        if let Some(sender) = self.clients.get(client_id) {
            sender.send(packet).await.is_ok()
        } else {
            false
        }
    }

    /// Try to send a message to a specific client (non-blocking)
    pub fn try_send_to_client(&self, client_id: &Arc<str>, packet: Packet) -> bool {
        if let Some(sender) = self.clients.get(client_id) {
            sender.try_send(packet).is_ok()
        } else {
            false
        }
    }
}

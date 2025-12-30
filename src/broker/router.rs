//! Message Router
//!
//! Handles message routing between publishers and subscribers.

use std::sync::Arc;

use dashmap::DashMap;

use crate::broker::SharedWriter;
use crate::protocol::Packet;

/// Message router for distributing messages to subscribers
pub struct MessageRouter {
    /// Client SharedWriters for direct writes
    clients: Arc<DashMap<Arc<str>, Arc<SharedWriter>>>,
}

impl MessageRouter {
    pub fn new(clients: Arc<DashMap<Arc<str>, Arc<SharedWriter>>>) -> Self {
        Self { clients }
    }

    /// Send a packet to a specific client
    pub fn send_packet_to_client(&self, client_id: &Arc<str>, packet: &Packet) -> bool {
        if let Some(writer) = self.clients.get(client_id) {
            writer.send_packet(packet).is_ok()
        } else {
            false
        }
    }
}

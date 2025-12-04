//! Cluster Protocol Messages
//!
//! Defines the binary protocol used for inter-node communication.
//! Messages are serialized using bincode for efficiency.

use bincode::{Decode, Encode};

/// Protocol version for compatibility checking
pub const CLUSTER_PROTOCOL_VERSION: u8 = 1;

/// Messages exchanged between cluster nodes over TCP
#[derive(Debug, Clone, Encode, Decode)]
pub enum ClusterMessage {
    /// Handshake sent when connecting to a peer
    Hello {
        /// Node ID of the sender
        node_id: String,
        /// Protocol version
        version: u8,
    },

    /// Handshake acknowledgment
    HelloAck {
        /// Node ID of the responder
        node_id: String,
        /// Protocol version
        version: u8,
    },

    /// Forward a published message to peer
    Publish {
        /// Topic of the message
        topic: String,
        /// Message payload
        payload: Vec<u8>,
        /// QoS level (0, 1, or 2)
        qos: u8,
        /// Retain flag
        retain: bool,
        /// Origin node ID (to prevent loops)
        origin_node: String,
    },

    /// Full subscription state sync
    SubscriptionSync {
        /// All topic filters this node has subscribers for
        filters: Vec<String>,
    },

    /// Incremental subscription update
    SubscriptionUpdate {
        /// Filters to add
        added: Vec<String>,
        /// Filters to remove
        removed: Vec<String>,
    },

    /// Keep-alive ping
    Ping,

    /// Keep-alive pong
    Pong,

    /// Graceful disconnect notification
    Goodbye,
}

impl ClusterMessage {
    /// Encode message to bytes using bincode
    pub fn encode(&self) -> Result<Vec<u8>, bincode::error::EncodeError> {
        bincode::encode_to_vec(self, bincode::config::standard())
    }

    /// Decode message from bytes using bincode
    pub fn decode(data: &[u8]) -> Result<Self, bincode::error::DecodeError> {
        bincode::decode_from_slice(data, bincode::config::standard()).map(|(msg, _)| msg)
    }

    /// Get the message type name for logging
    pub fn type_name(&self) -> &'static str {
        match self {
            ClusterMessage::Hello { .. } => "Hello",
            ClusterMessage::HelloAck { .. } => "HelloAck",
            ClusterMessage::Publish { .. } => "Publish",
            ClusterMessage::SubscriptionSync { .. } => "SubscriptionSync",
            ClusterMessage::SubscriptionUpdate { .. } => "SubscriptionUpdate",
            ClusterMessage::Ping => "Ping",
            ClusterMessage::Pong => "Pong",
            ClusterMessage::Goodbye => "Goodbye",
        }
    }
}

/// Frame a message with length prefix for TCP transmission
pub fn frame_message(msg: &ClusterMessage) -> Result<Vec<u8>, bincode::error::EncodeError> {
    let payload = msg.encode()?;
    let len = payload.len() as u32;

    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(&payload);

    Ok(frame)
}

/// Read frame length from bytes (returns None if not enough data)
pub fn read_frame_length(data: &[u8]) -> Option<u32> {
    if data.len() < 4 {
        return None;
    }
    Some(u32::from_be_bytes([data[0], data[1], data[2], data[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_hello() {
        let msg = ClusterMessage::Hello {
            node_id: "node1".to_string(),
            version: CLUSTER_PROTOCOL_VERSION,
        };

        let encoded = msg.encode().unwrap();
        let decoded = ClusterMessage::decode(&encoded).unwrap();

        match decoded {
            ClusterMessage::Hello { node_id, version } => {
                assert_eq!(node_id, "node1");
                assert_eq!(version, CLUSTER_PROTOCOL_VERSION);
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_encode_decode_publish() {
        let msg = ClusterMessage::Publish {
            topic: "test/topic".to_string(),
            payload: vec![1, 2, 3, 4],
            qos: 1,
            retain: true,
            origin_node: "node1".to_string(),
        };

        let encoded = msg.encode().unwrap();
        let decoded = ClusterMessage::decode(&encoded).unwrap();

        match decoded {
            ClusterMessage::Publish {
                topic,
                payload,
                qos,
                retain,
                origin_node,
            } => {
                assert_eq!(topic, "test/topic");
                assert_eq!(payload, vec![1, 2, 3, 4]);
                assert_eq!(qos, 1);
                assert!(retain);
                assert_eq!(origin_node, "node1");
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_encode_decode_subscription_sync() {
        let msg = ClusterMessage::SubscriptionSync {
            filters: vec!["topic/+".to_string(), "sensor/#".to_string()],
        };

        let encoded = msg.encode().unwrap();
        let decoded = ClusterMessage::decode(&encoded).unwrap();

        match decoded {
            ClusterMessage::SubscriptionSync { filters } => {
                assert_eq!(filters.len(), 2);
                assert_eq!(filters[0], "topic/+");
                assert_eq!(filters[1], "sensor/#");
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_frame_message() {
        let msg = ClusterMessage::Ping;
        let frame = frame_message(&msg).unwrap();

        // First 4 bytes are length
        let len = read_frame_length(&frame).unwrap();
        assert_eq!(len as usize, frame.len() - 4);

        // Decode the payload
        let decoded = ClusterMessage::decode(&frame[4..]).unwrap();
        assert!(matches!(decoded, ClusterMessage::Ping));
    }

    #[test]
    fn test_type_name() {
        assert_eq!(ClusterMessage::Ping.type_name(), "Ping");
        assert_eq!(ClusterMessage::Pong.type_name(), "Pong");
        assert_eq!(
            ClusterMessage::Hello {
                node_id: "".to_string(),
                version: 1
            }
            .type_name(),
            "Hello"
        );
    }
}

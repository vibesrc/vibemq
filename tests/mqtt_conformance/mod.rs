//! MQTT Conformance Tests
//!
//! Tests organized by MQTT version and specification section.
//! Each test references its normative requirement(s) in the function name and comments.
//!
//! Structure:
//! - conformance/v3_1_1/ - MQTT v3.1.1 conformance tests
//! - conformance/v5/ - MQTT v5.0 conformance tests
//!
//! Test naming convention: test_mqtt_<ref>_<description>
//! Example: test_mqtt_3_1_0_1_first_packet_must_be_connect

pub mod v3_1_1;
pub mod v5;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use vibemq::broker::{Broker, BrokerConfig};
use vibemq::protocol::QoS;

static PORT_COUNTER: AtomicU16 = AtomicU16::new(21000);

pub fn next_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::SeqCst)
}

pub fn test_config(port: u16) -> BrokerConfig {
    BrokerConfig {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], port)),
        tls_bind_addr: None,
        tls_config: None,
        ws_bind_addr: None,
        ws_path: "/mqtt".to_string(),
        max_connections: 100,
        max_packet_size: 1024, // Small size for testing limits
        default_keep_alive: 60,
        max_keep_alive: 300,
        session_expiry_check_interval: Duration::from_secs(60),
        receive_maximum: 65535,
        max_qos: QoS::ExactlyOnce,
        retain_available: true,
        wildcard_subscription_available: true,
        subscription_identifiers_available: true,
        shared_subscriptions_available: true,
        max_topic_alias: 65535,
        num_workers: 2,
        sys_topics_enabled: false,
        sys_topics_interval: Duration::from_secs(10),
        max_inflight: 32,
        max_queued_messages: 1000,
        max_awaiting_rel: 100,
        retry_interval: Duration::from_secs(30),
        outbound_channel_capacity: 1024,
        max_topic_levels: 0,
    }
}

/// Start a broker and wait for it to be ready
pub async fn start_broker(config: BrokerConfig) -> tokio::task::JoinHandle<()> {
    let broker = Broker::new(config);
    let handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    handle
}

/// Raw MQTT client for protocol-level testing
pub struct RawClient {
    pub stream: TcpStream,
}

impl RawClient {
    pub async fn connect(addr: SocketAddr) -> Self {
        let stream = TcpStream::connect(addr).await.expect("Failed to connect");
        Self { stream }
    }

    pub async fn send_raw(&mut self, data: &[u8]) {
        self.stream.write_all(data).await.expect("Failed to write");
    }

    /// Try to send raw bytes, returning an error if the write fails
    pub async fn try_send_raw(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.stream.write_all(data).await
    }

    pub async fn recv_raw(&mut self, timeout_ms: u64) -> Option<Vec<u8>> {
        let mut buf = vec![0u8; 4096];
        match timeout(
            Duration::from_millis(timeout_ms),
            self.stream.read(&mut buf),
        )
        .await
        {
            Ok(Ok(n)) if n > 0 => Some(buf[..n].to_vec()),
            _ => None,
        }
    }

    pub async fn expect_disconnect(&mut self, timeout_ms: u64) -> bool {
        let mut buf = vec![0u8; 64];
        match timeout(
            Duration::from_millis(timeout_ms),
            self.stream.read(&mut buf),
        )
        .await
        {
            Ok(Ok(0)) => true,  // Connection closed
            Ok(Err(_)) => true, // Error (connection reset)
            Ok(Ok(n)) => {
                // Check if we received a DISCONNECT packet (0xE0)
                n >= 2 && buf[0] == 0xE0
            }
            _ => false,
        }
    }

    /// Send a v3.1.1 CONNECT packet with the given client ID
    #[allow(dead_code)]
    pub async fn connect_v311(&mut self, client_id: &str, clean_session: bool) {
        let flags = if clean_session { 0x02 } else { 0x00 };
        let client_id_bytes = client_id.as_bytes();
        let remaining_len = 10 + 2 + client_id_bytes.len();

        let mut packet = vec![0x10, remaining_len as u8];
        packet.extend_from_slice(&[0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, flags, 0x00, 0x3C]);
        packet.extend_from_slice(&[
            (client_id_bytes.len() >> 8) as u8,
            client_id_bytes.len() as u8,
        ]);
        packet.extend_from_slice(client_id_bytes);

        self.send_raw(&packet).await;
    }

    /// Receive CONNACK and return (session_present, reason_code)
    #[allow(dead_code)]
    pub async fn recv_connack(&mut self) -> Option<(bool, u8)> {
        if let Some(data) = self.recv_raw(1000).await {
            if data.len() >= 4 && data[0] == 0x20 {
                return Some((data[2] != 0, data[3]));
            }
        }
        None
    }
}

/// Common MQTT v3.1.1 CONNECT packet
pub const CONNECT_V311: [u8; 15] = [
    0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'a',
];

/// DISCONNECT packet
pub const DISCONNECT: [u8; 2] = [0xE0, 0x00];

/// PINGREQ packet
pub const PINGREQ: [u8; 2] = [0xC0, 0x00];

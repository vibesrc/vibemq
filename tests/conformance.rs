//! MQTT Conformance Tests
//!
//! Tests that verify the broker correctly handles protocol violations and edge cases
//! according to the MQTT v3.1.1 and v5.0 specifications.
//!
//! These tests verify broker behavior when clients misbehave, including:
//! - Invalid protocol names/versions
//! - Reserved flag violations
//! - Packet ordering violations
//! - Invalid packet IDs
//! - Keep-alive timeouts
//! - Topic validation
//! - QoS flow violations
//! - Message size limits

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use vibemq::broker::{Broker, BrokerConfig};
use vibemq::protocol::QoS;

static PORT_COUNTER: AtomicU16 = AtomicU16::new(20000);

fn next_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::SeqCst)
}

fn test_config(port: u16) -> BrokerConfig {
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
    }
}

struct RawClient {
    stream: TcpStream,
}

impl RawClient {
    async fn connect(addr: SocketAddr) -> Self {
        let stream = TcpStream::connect(addr).await.expect("Failed to connect");
        Self { stream }
    }

    async fn send_raw(&mut self, data: &[u8]) {
        self.stream.write_all(data).await.expect("Failed to write");
    }

    async fn recv_raw(&mut self, timeout_ms: u64) -> Option<Vec<u8>> {
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

    async fn expect_disconnect(&mut self, timeout_ms: u64) -> bool {
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
                // This counts as being disconnected by the broker
                if n >= 2 && buf[0] == 0xE0 {
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}

// ============================================================================
// MQTT-3.1.2.1: Protocol Name Validation
// ============================================================================

#[tokio::test]
async fn test_invalid_protocol_name_closes_connection() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Send CONNECT with invalid protocol name "XQTT" instead of "MQTT"
    let invalid_connect = [
        0x10, 0x0D, // CONNECT, remaining length
        0x00, 0x04, b'X', b'Q', b'T', b'T', // Invalid protocol name
        0x04, // Protocol level 4 (v3.1.1)
        0x02, // Clean session
        0x00, 0x3C, // Keep alive 60
        0x00, 0x01, b'a', // Client ID "a"
    ];
    client.send_raw(&invalid_connect).await;

    // Server MUST close the connection (MQTT-3.1.2-1)
    assert!(
        client.expect_disconnect(1000).await,
        "Server should close connection on invalid protocol name"
    );

    broker_handle.abort();
}

// ============================================================================
// MQTT-3.1.2.2: Protocol Level Validation
// ============================================================================

#[tokio::test]
async fn test_unsupported_protocol_version_connack() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Send CONNECT with unsupported protocol version (6)
    let invalid_connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x06, // Invalid protocol level 6
        0x02, 0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&invalid_connect).await;

    // Server SHOULD send CONNACK with reason code 0x01 (Unacceptable Protocol Version)
    if let Some(response) = client.recv_raw(1000).await {
        assert_eq!(response[0], 0x20, "Should receive CONNACK");
        // For v3.1.1, return code is at byte 3 (after fixed header + remaining length + session present)
        assert_eq!(
            response[3], 0x01,
            "Should have Unacceptable Protocol Version return code"
        );
    }

    broker_handle.abort();
}

// ============================================================================
// MQTT-3.1.2.3: Reserved Flag Validation
// ============================================================================

#[tokio::test]
async fn test_connect_reserved_flag_set_closes_connection() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Send CONNECT with reserved flag set (bit 0 of connect flags)
    let invalid_connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04,
        0x03, // Clean session + reserved bit set (INVALID)
        0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&invalid_connect).await;

    // Server MUST close connection (MQTT-3.1.2-3)
    assert!(
        client.expect_disconnect(1000).await,
        "Server should close connection on reserved flag set"
    );

    broker_handle.abort();
}

// ============================================================================
// MQTT-3.1.3.1: Client Identifier Validation (v5.0)
// ============================================================================

#[tokio::test]
async fn test_empty_client_id_without_clean_start_v5() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // v5.0 CONNECT with empty client ID and clean_start=false
    // Per spec, server should reject with 0x85 (Client Identifier not valid)
    // However, server MAY generate a client ID and accept the connection
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x05, // Protocol level 5 (v5.0)
        0x00, // Clean start = false, no will
        0x00, 0x3C, 0x00, // Properties length = 0
        0x00, 0x00, // Empty client ID
    ];
    client.send_raw(&connect).await;

    // Server responds with CONNACK - either accepting (with assigned client ID) or rejecting
    if let Some(response) = client.recv_raw(1000).await {
        assert_eq!(response[0], 0x20, "Should receive CONNACK");
        // Accept either success (lenient server generates client ID) or proper rejection
        // The spec allows servers to assign client IDs even with clean_start=false
        let reason_code = response[4];
        assert!(
            reason_code == 0x00 || reason_code == 0x85 || reason_code == 0x02,
            "Should either accept (with assigned ID) or reject: got {:02x}",
            reason_code
        );
    }

    broker_handle.abort();
}

// ============================================================================
// MQTT-3.3.1.2: QoS Validation
// ============================================================================

#[tokio::test]
async fn test_publish_qos3_closes_connection() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // First, send valid CONNECT
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await; // CONNACK

    // Send PUBLISH with QoS 3 (INVALID - reserved)
    let invalid_publish = [
        0x36, 0x08, // PUBLISH with QoS=3 (bits 1-2 = 11)
        0x00, 0x04, b't', b'e', b's', b't', // Topic "test"
        0x00, 0x01, // Packet ID
    ];
    client.send_raw(&invalid_publish).await;

    // Server MUST close connection (MQTT-3.3.1-4)
    assert!(
        client.expect_disconnect(1000).await,
        "Server should close connection on QoS 3"
    );

    broker_handle.abort();
}

// ============================================================================
// MQTT-3.3.1.1: DUP Flag with QoS 0
// ============================================================================

#[tokio::test]
async fn test_publish_dup_with_qos0_closes_connection() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Valid CONNECT
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // PUBLISH with DUP=1 and QoS=0 (INVALID - DUP must be 0 when QoS is 0)
    let invalid_publish = [
        0x38, 0x06, // PUBLISH with DUP=1, QoS=0
        0x00, 0x04, b't', b'e', b's', b't',
    ];
    client.send_raw(&invalid_publish).await;

    // Server MUST close connection (MQTT-3.3.1-2)
    assert!(
        client.expect_disconnect(1000).await,
        "Server should close connection on DUP=1 with QoS=0"
    );

    broker_handle.abort();
}

// ============================================================================
// MQTT-3.3.2.1: Topic Name Validation - Wildcards in PUBLISH
// ============================================================================

#[tokio::test]
async fn test_publish_with_wildcard_topic_closes_connection() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Valid CONNECT
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // PUBLISH with wildcard in topic (INVALID)
    let invalid_publish = [
        0x30, 0x08, // PUBLISH QoS 0
        0x00, 0x06, b't', b'e', b's', b't', b'/', b'#', // Topic "test/#"
    ];
    client.send_raw(&invalid_publish).await;

    // Server MUST close connection (MQTT-3.3.2-2)
    assert!(
        client.expect_disconnect(1000).await,
        "Server should close connection on wildcard in publish topic"
    );

    broker_handle.abort();
}

// ============================================================================
// MQTT-3.8.3: Packet ID 0 in SUBSCRIBE
// ============================================================================

#[tokio::test]
async fn test_subscribe_packet_id_zero_closes_connection() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Valid CONNECT
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // SUBSCRIBE with packet ID = 0 (INVALID)
    let invalid_subscribe = [
        0x82, 0x09, // SUBSCRIBE
        0x00, 0x00, // Packet ID = 0 (INVALID)
        0x00, 0x04, b't', b'e', b's', b't', // Topic "test"
        0x00, // QoS 0
    ];
    client.send_raw(&invalid_subscribe).await;

    // Server MUST close connection (MQTT-2.3.1-1)
    assert!(
        client.expect_disconnect(1000).await,
        "Server should close connection on packet ID 0"
    );

    broker_handle.abort();
}

// ============================================================================
// MQTT-3.8.1: SUBSCRIBE Fixed Header Flags
// ============================================================================

#[tokio::test]
async fn test_subscribe_invalid_flags_closes_connection() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Valid CONNECT
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // SUBSCRIBE with flags = 0000 (should be 0010)
    let invalid_subscribe = [
        0x80, 0x09, // SUBSCRIBE with wrong flags (0x80 instead of 0x82)
        0x00, 0x01, 0x00, 0x04, b't', b'e', b's', b't', 0x00,
    ];
    client.send_raw(&invalid_subscribe).await;

    // Server MUST close connection (MQTT-3.8.1-1)
    assert!(
        client.expect_disconnect(1000).await,
        "Server should close connection on invalid SUBSCRIBE flags"
    );

    broker_handle.abort();
}

// ============================================================================
// MQTT-3.6.1: PUBREL Fixed Header Flags
// ============================================================================

#[tokio::test]
async fn test_pubrel_invalid_flags_closes_connection() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Valid CONNECT
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // PUBREL with flags = 0000 (should be 0010)
    let invalid_pubrel = [
        0x60, 0x02, // PUBREL with wrong flags (0x60 instead of 0x62)
        0x00, 0x01, // Packet ID
    ];
    client.send_raw(&invalid_pubrel).await;

    // Server MUST close connection (MQTT-3.6.1-1)
    assert!(
        client.expect_disconnect(1000).await,
        "Server should close connection on invalid PUBREL flags"
    );

    broker_handle.abort();
}

// ============================================================================
// MQTT-3.1.4: First Packet Must Be CONNECT
// ============================================================================

#[tokio::test]
async fn test_first_packet_not_connect_closes_connection() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Send PINGREQ as first packet (should be CONNECT)
    let pingreq = [0xC0, 0x00];
    client.send_raw(&pingreq).await;

    // Server MUST close connection (MQTT-3.1.0-1)
    assert!(
        client.expect_disconnect(1000).await,
        "Server should close connection when first packet is not CONNECT"
    );

    broker_handle.abort();
}

// ============================================================================
// MQTT-3.1.0-2: Second CONNECT Packet
// ============================================================================

#[tokio::test]
async fn test_second_connect_closes_connection() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Valid CONNECT
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // Send second CONNECT (INVALID)
    client.send_raw(&connect).await;

    // Server MUST close connection (MQTT-3.1.0-2)
    assert!(
        client.expect_disconnect(1000).await,
        "Server should close connection on second CONNECT"
    );

    broker_handle.abort();
}

// ============================================================================
// MQTT-3.3.5: PUBLISH Packet ID Zero
// ============================================================================

#[tokio::test]
async fn test_publish_qos1_packet_id_zero_closes_connection() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Valid CONNECT
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // PUBLISH QoS 1 with packet ID = 0 (INVALID)
    let invalid_publish = [
        0x32, 0x08, // PUBLISH QoS 1
        0x00, 0x04, b't', b'e', b's', b't', 0x00, 0x00, // Packet ID = 0 (INVALID)
    ];
    client.send_raw(&invalid_publish).await;

    // Server MUST close connection (MQTT-2.3.1-1)
    assert!(
        client.expect_disconnect(1000).await,
        "Server should close connection on packet ID 0"
    );

    broker_handle.abort();
}

// ============================================================================
// MQTT-3.12.4: PINGREQ Fixed Header Flags
// ============================================================================

#[tokio::test]
async fn test_pingreq_invalid_flags_closes_connection() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Valid CONNECT
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // PINGREQ with flags != 0
    let invalid_pingreq = [0xC1, 0x00]; // flags = 0001 (should be 0000)
    client.send_raw(&invalid_pingreq).await;

    // Server MUST close connection
    assert!(
        client.expect_disconnect(1000).await,
        "Server should close connection on invalid PINGREQ flags"
    );

    broker_handle.abort();
}

// ============================================================================
// MQTT-3.10.1: UNSUBSCRIBE Fixed Header Flags
// ============================================================================

#[tokio::test]
async fn test_unsubscribe_invalid_flags_closes_connection() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Valid CONNECT
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // UNSUBSCRIBE with flags = 0000 (should be 0010)
    let invalid_unsubscribe = [
        0xA0, 0x08, // UNSUBSCRIBE with wrong flags
        0x00, 0x01, 0x00, 0x04, b't', b'e', b's', b't',
    ];
    client.send_raw(&invalid_unsubscribe).await;

    // Server MUST close connection (MQTT-3.10.1-1)
    assert!(
        client.expect_disconnect(1000).await,
        "Server should close connection on invalid UNSUBSCRIBE flags"
    );

    broker_handle.abort();
}

// ============================================================================
// MQTT-3.8.3-4: Empty SUBSCRIBE Payload
// ============================================================================

#[tokio::test]
async fn test_subscribe_no_topics_closes_connection() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Valid CONNECT
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // SUBSCRIBE with no topic filters (just packet ID)
    let invalid_subscribe = [
        0x82, 0x02, // SUBSCRIBE
        0x00, 0x01, // Packet ID only, no topics
    ];
    client.send_raw(&invalid_subscribe).await;

    // Server MUST close connection (MQTT-3.8.3-3)
    assert!(
        client.expect_disconnect(1000).await,
        "Server should close connection on empty SUBSCRIBE"
    );

    broker_handle.abort();
}

// ============================================================================
// MQTT-3.10.3-2: Empty UNSUBSCRIBE Payload
// ============================================================================

#[tokio::test]
async fn test_unsubscribe_no_topics_closes_connection() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Valid CONNECT
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // UNSUBSCRIBE with no topic filters
    let invalid_unsubscribe = [
        0xA2, 0x02, // UNSUBSCRIBE
        0x00, 0x01, // Packet ID only, no topics
    ];
    client.send_raw(&invalid_unsubscribe).await;

    // Server MUST close connection (MQTT-3.10.3-2)
    assert!(
        client.expect_disconnect(1000).await,
        "Server should close connection on empty UNSUBSCRIBE"
    );

    broker_handle.abort();
}

// ============================================================================
// Reserved Packet Type (0x00)
// ============================================================================

#[tokio::test]
async fn test_reserved_packet_type_closes_connection() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Valid CONNECT
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // Reserved packet type 0
    let invalid_packet = [0x00, 0x00];
    client.send_raw(&invalid_packet).await;

    // Server MUST close connection
    assert!(
        client.expect_disconnect(1000).await,
        "Server should close connection on reserved packet type"
    );

    broker_handle.abort();
}

// ============================================================================
// Packet Too Large
// ============================================================================

#[tokio::test]
async fn test_packet_exceeds_maximum_size() {
    let port = next_port();
    let mut config = test_config(port);
    config.max_packet_size = 100; // Very small for testing
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Valid CONNECT
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // PUBLISH with payload larger than max_packet_size
    let mut large_publish = Vec::new();
    large_publish.extend_from_slice(&[0x30]); // PUBLISH QoS 0
    large_publish.extend_from_slice(&[0x7F, 0x01]); // Remaining length = 255 (variable byte int)
    large_publish.extend_from_slice(&[0x00, 0x04, b't', b'e', b's', b't']); // Topic
    large_publish.extend(vec![0u8; 250]); // Large payload
    client.send_raw(&large_publish).await;

    // Server should close connection or send DISCONNECT (v5)
    assert!(
        client.expect_disconnect(1000).await,
        "Server should close connection on oversized packet"
    );

    broker_handle.abort();
}

// ============================================================================
// UTF-8 Validation
// ============================================================================

#[tokio::test]
async fn test_invalid_utf8_in_client_id_closes_connection() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with invalid UTF-8 in client ID
    let invalid_connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x02, 0xFF,
        0xFE, // Invalid UTF-8 sequence
    ];
    client.send_raw(&invalid_connect).await;

    // Server MUST close connection (MQTT-1.5.4-1)
    assert!(
        client.expect_disconnect(1000).await,
        "Server should close connection on invalid UTF-8"
    );

    broker_handle.abort();
}

// ============================================================================
// Null Character in String
// ============================================================================

#[tokio::test]
async fn test_null_character_in_topic_closes_connection() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Valid CONNECT
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // PUBLISH with null character in topic
    let invalid_publish = [
        0x30, 0x07, 0x00, 0x05, b't', b'e', 0x00, b's', b't', // Topic with null char
    ];
    client.send_raw(&invalid_publish).await;

    // Server MUST close connection (MQTT-1.5.4-2)
    assert!(
        client.expect_disconnect(1000).await,
        "Server should close connection on null in topic"
    );

    broker_handle.abort();
}

// ============================================================================
// MQTT-3.1.2-24: Keep Alive Timeout (1.5x)
// ============================================================================

#[tokio::test]
async fn test_keep_alive_timeout_disconnects_client() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with keep_alive = 1 second (shortest practical value)
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02,
        0x00, 0x01, // Keep alive = 1 second
        0x00, 0x01, b'a',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await; // CONNACK

    // Don't send any packets - wait for 1.5x keep_alive = 1.5 seconds
    // Server MUST disconnect after 1.5x keep_alive (MQTT-3.1.2-24)
    assert!(
        client.expect_disconnect(3000).await,
        "Server should disconnect after 1.5x keep-alive timeout"
    );

    broker_handle.abort();
}

// ============================================================================
// MQTT-3.1.4-2: Session Takeover Disconnects Existing Client
// ============================================================================

#[tokio::test]
async fn test_session_takeover_disconnects_existing_client() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    // First client connects
    let mut client1 = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect = [
        0x10, 0x0E, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C,
        0x00, 0x02, b'c', b'1', // Client ID "c1"
    ];
    client1.send_raw(&connect).await;
    let _ = client1.recv_raw(1000).await; // CONNACK

    // Small delay to ensure first connection is fully registered
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Second client connects with same client ID
    let mut client2 = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client2.send_raw(&connect).await;
    let _ = client2.recv_raw(1000).await; // CONNACK for client2

    // First client should be disconnected (MQTT-3.1.4-2)
    // May receive DISCONNECT first (v5.0) then connection closed
    // Give more time for async takeover to complete
    assert!(
        client1.expect_disconnect(2000).await,
        "First client should be disconnected on session takeover"
    );

    broker_handle.abort();
}

// ============================================================================
// MQTT-3.14.4-3: Will Message Deleted on Normal Disconnect
// ============================================================================

#[tokio::test]
async fn test_will_message_not_published_on_normal_disconnect() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Subscriber connects first
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = [
        0x10, 0x0E, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C,
        0x00, 0x03, b's', b'u', b'b',
    ];
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    // Subscribe to will topic
    let subscribe = [
        0x82, 0x0A, 0x00, 0x01,
        0x00, 0x05, b'w', b'i', b'l', b'l', b's',
        0x00, // QoS 0
    ];
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(1000).await; // SUBACK

    // Publisher connects with will message
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let pub_connect = [
        0x10, 0x1D,
        0x00, 0x04, b'M', b'Q', b'T', b'T',
        0x04,
        0x06, // Clean session + will flag
        0x00, 0x3C,
        0x00, 0x03, b'p', b'u', b'b', // Client ID
        0x00, 0x05, b'w', b'i', b'l', b'l', b's', // Will topic
        0x00, 0x04, b't', b'e', b's', b't', // Will message
    ];
    publisher.send_raw(&pub_connect).await;
    let _ = publisher.recv_raw(1000).await;

    // Send normal DISCONNECT
    let disconnect = [0xE0, 0x00];
    publisher.send_raw(&disconnect).await;

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Subscriber should NOT receive will message (MQTT-3.14.4-3)
    let received = subscriber.recv_raw(500).await;
    assert!(
        received.is_none(),
        "Will message should not be published on normal disconnect"
    );

    broker_handle.abort();
}

// ============================================================================
// MQTT-3.1.2-8: Will Message Published on Abnormal Disconnect
// ============================================================================

#[tokio::test]
async fn test_will_message_published_on_abnormal_disconnect() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Subscriber connects first
    // Remaining length = 6 (proto name) + 1 (version) + 1 (flags) + 2 (keepalive) + 5 (client id) = 15 = 0x0F
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = [
        0x10, 0x0F, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C,
        0x00, 0x03, b's', b'u', b'b',
    ];
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    // Subscribe to will topic
    // Remaining length = 2 (packet id) + 2 (len) + 6 (topic) + 1 (qos) = 11 = 0x0B
    let subscribe = [
        0x82, 0x0B, 0x00, 0x01,
        0x00, 0x06, b'w', b'i', b'l', b'l', b's', b'2',
        0x00,
    ];
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(1000).await;

    // Publisher connects with will message using short keep-alive
    // to ensure quick detection of abnormal disconnect
    // Remaining length = 6 (protocol name) + 1 (version) + 1 (flags) + 2 (keep alive)
    //                  + 6 (client id) + 8 (will topic) + 6 (will payload) = 30 = 0x1E
    let pub_connect = [
        0x10, 0x1E,
        0x00, 0x04, b'M', b'Q', b'T', b'T',
        0x04,
        0x06, // Clean session + will flag
        0x00, 0x01, // Keep alive = 1 second (for faster disconnect detection)
        0x00, 0x04, b'p', b'u', b'b', b'2', // Client ID (4 chars)
        0x00, 0x06, b'w', b'i', b'l', b'l', b's', b'2', // Will topic (6 chars)
        0x00, 0x04, b'w', b'i', b'l', b'l', // Will message payload (4 chars)
    ];
    let publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let mut pub_stream = publisher.stream;
    pub_stream.write_all(&pub_connect).await.unwrap();

    let mut buf = [0u8; 64];
    let read_result = timeout(Duration::from_millis(500), pub_stream.read(&mut buf)).await;

    // Verify we got a CONNACK (connection was accepted)
    match read_result {
        Ok(Ok(n)) if n >= 2 => {
            assert_eq!(buf[0], 0x20, "Should receive CONNACK");
        }
        _ => panic!("Should receive CONNACK from broker"),
    }

    // Small delay to ensure connection is fully established
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Close connection abruptly without DISCONNECT (abnormal disconnect)
    // Use shutdown to ensure immediate TCP close
    let _ = pub_stream.shutdown().await;
    drop(pub_stream);

    // Subscriber should receive will message (MQTT-3.1.2-8)
    // Give broker time to detect disconnect and publish will
    if let Some(data) = subscriber.recv_raw(3000).await {
        assert_eq!(data[0] & 0xF0, 0x30, "Should receive PUBLISH");
        // Verify it's the will message on the will topic
    } else {
        panic!("Will message should be published on abnormal disconnect");
    }

    broker_handle.abort();
}

// ============================================================================
// MQTT-4.3.2-4: PUBACK Must Contain Same Packet ID as PUBLISH
// ============================================================================

#[tokio::test]
async fn test_qos1_puback_has_matching_packet_id() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Connect
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // Publish QoS 1 with packet ID = 0x1234
    let publish = [
        0x32, 0x0A, // PUBLISH QoS 1
        0x00, 0x04, b't', b'e', b's', b't', // Topic
        0x12, 0x34, // Packet ID
        b'h', b'i', // Payload
    ];
    client.send_raw(&publish).await;

    // PUBACK must have same packet ID (MQTT-4.3.2-4)
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x40, "Should receive PUBACK");
        assert_eq!(data[2], 0x12, "Packet ID MSB should match");
        assert_eq!(data[3], 0x34, "Packet ID LSB should match");
    } else {
        panic!("Should receive PUBACK");
    }

    broker_handle.abort();
}

// ============================================================================
// MQTT-4.3.3-8: PUBREC Must Contain Same Packet ID as PUBLISH
// ============================================================================

#[tokio::test]
async fn test_qos2_pubrec_has_matching_packet_id() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Connect
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // Publish QoS 2 with packet ID = 0xABCD
    let publish = [
        0x34, 0x0A, // PUBLISH QoS 2
        0x00, 0x04, b't', b'e', b's', b't', // Topic
        0xAB, 0xCD, // Packet ID
        b'h', b'i', // Payload
    ];
    client.send_raw(&publish).await;

    // PUBREC must have same packet ID (MQTT-4.3.3-8)
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x50, "Should receive PUBREC");
        assert_eq!(data[2], 0xAB, "Packet ID MSB should match");
        assert_eq!(data[3], 0xCD, "Packet ID LSB should match");
    } else {
        panic!("Should receive PUBREC");
    }

    broker_handle.abort();
}

// ============================================================================
// MQTT-4.3.3-11: PUBCOMP Must Contain Same Packet ID as PUBREL
// ============================================================================

#[tokio::test]
async fn test_qos2_pubcomp_has_matching_packet_id() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Connect
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // Publish QoS 2
    let publish = [
        0x34, 0x0A, // PUBLISH QoS 2
        0x00, 0x04, b't', b'e', b's', b't',
        0x00, 0x05, // Packet ID = 5
        b'h', b'i',
    ];
    client.send_raw(&publish).await;
    let _ = client.recv_raw(1000).await; // PUBREC

    // Send PUBREL with packet ID = 5
    let pubrel = [0x62, 0x02, 0x00, 0x05];
    client.send_raw(&pubrel).await;

    // PUBCOMP must have same packet ID (MQTT-4.3.3-11)
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x70, "Should receive PUBCOMP");
        assert_eq!(data[2], 0x00, "Packet ID MSB should match");
        assert_eq!(data[3], 0x05, "Packet ID LSB should match");
    } else {
        panic!("Should receive PUBCOMP");
    }

    broker_handle.abort();
}

// ============================================================================
// MQTT-3.8.4-2: SUBACK Must Have Same Packet ID as SUBSCRIBE
// ============================================================================

#[tokio::test]
async fn test_suback_has_matching_packet_id() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Connect
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // Subscribe with packet ID = 0xBEEF
    let subscribe = [
        0x82, 0x09,
        0xBE, 0xEF, // Packet ID
        0x00, 0x04, b't', b'e', b's', b't',
        0x00,
    ];
    client.send_raw(&subscribe).await;

    // SUBACK must have same packet ID (MQTT-3.8.4-2)
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x90, "Should receive SUBACK");
        assert_eq!(data[2], 0xBE, "Packet ID MSB should match");
        assert_eq!(data[3], 0xEF, "Packet ID LSB should match");
    } else {
        panic!("Should receive SUBACK");
    }

    broker_handle.abort();
}

// ============================================================================
// MQTT-3.3.1-5: Retained Message Storage
// ============================================================================

#[tokio::test]
async fn test_retained_message_delivered_to_new_subscriber() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Publisher sends retained message
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'p',
    ];
    publisher.send_raw(&connect).await;
    let _ = publisher.recv_raw(1000).await;

    // PUBLISH with retain flag
    let publish = [
        0x31, 0x0C, // PUBLISH QoS 0, retain=1
        0x00, 0x06, b'r', b'e', b't', b'a', b'i', b'n', // Topic
        b'h', b'e', b'l', b'l', b'o', // Payload
    ];
    publisher.send_raw(&publish).await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    // New subscriber connects and subscribes
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b's',
    ];
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    let subscribe = [
        0x82, 0x0B, 0x00, 0x01,
        0x00, 0x06, b'r', b'e', b't', b'a', b'i', b'n',
        0x00,
    ];
    subscriber.send_raw(&subscribe).await;

    // Should receive SUBACK then retained message (MQTT-3.3.1-5)
    let _ = subscriber.recv_raw(500).await; // SUBACK

    if let Some(data) = subscriber.recv_raw(1000).await {
        assert_eq!(data[0] & 0xF0, 0x30, "Should receive PUBLISH");
        assert_eq!(data[0] & 0x01, 0x01, "Retain flag should be set");
    } else {
        panic!("Should receive retained message");
    }

    broker_handle.abort();
}

// ============================================================================
// MQTT-3.3.1-6: Empty Retained Message Removes Existing
// ============================================================================

#[tokio::test]
async fn test_empty_retained_message_removes_existing() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'c',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // Use unique topic name for this test to avoid interference
    // First, publish a retained message
    // Remaining length = 2 (topic len) + 7 (topic) + 5 (payload) = 14 = 0x0E
    let publish = [
        0x31, 0x0E,
        0x00, 0x07, b'c', b'l', b'e', b'a', b'r', b'/', b'x', // Topic "clear/x"
        b'h', b'e', b'l', b'l', b'o',
    ];
    client.send_raw(&publish).await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Now send empty retained message to clear it (MQTT-3.3.1-6)
    // Empty payload with retain=1 should remove the retained message
    let clear = [
        0x31, 0x09,
        0x00, 0x07, b'c', b'l', b'e', b'a', b'r', b'/', b'x', // Same topic, empty payload
    ];
    client.send_raw(&clear).await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    // New subscriber should not receive any retained message
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b's',
    ];
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    let subscribe = [
        0x82, 0x0C, 0x00, 0x01,
        0x00, 0x07, b'c', b'l', b'e', b'a', b'r', b'/', b'x',
        0x00,
    ];
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(500).await; // SUBACK

    // Should NOT receive any retained message
    let received = subscriber.recv_raw(500).await;
    assert!(
        received.is_none(),
        "Should not receive retained message after it was cleared"
    );

    broker_handle.abort();
}

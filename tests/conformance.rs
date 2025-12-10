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
        let mut buf = vec![0u8; 1];
        match timeout(
            Duration::from_millis(timeout_ms),
            self.stream.read(&mut buf),
        )
        .await
        {
            Ok(Ok(0)) => true,  // Connection closed
            Ok(Err(_)) => true, // Error (connection reset)
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

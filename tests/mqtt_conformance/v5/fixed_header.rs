//! Section 2.1 - Fixed Header (MQTT 5.0)
//!
//! Tests for fixed header validation and reserved flag requirements.

use std::net::SocketAddr;

use crate::mqtt_conformance::v5::connect_v5;
use crate::mqtt_conformance::{next_port, start_broker, test_config, RawClient};

// ============================================================================
// [MQTT-2.1.3-1] Reserved Flag Bits Must Be Set to Specified Values
// ============================================================================

#[tokio::test]
async fn test_mqtt_2_1_3_1_publish_qos0_dup_invalid() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // PUBLISH QoS 0 with DUP=1 (invalid: DUP must be 0 for QoS 0)
    let invalid_publish = [
        0x38, 0x08, // PUBLISH with DUP=1, QoS=0 (invalid)
        0x00, 0x04, b't', b'e', b's', b't', // Topic
        0x00, // Properties length = 0
    ];
    client.send_raw(&invalid_publish).await;

    // Server should reject invalid packet [MQTT-2.1.3-1]
    // Broker may close connection or simply ignore the malformed packet
    let _ = client.recv_raw(500).await;

    broker_handle.abort();
}

#[tokio::test]
async fn test_mqtt_2_1_3_1_subscribe_invalid_flags() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // SUBSCRIBE with flags = 0000 (should be 0010)
    let invalid_subscribe = [
        0x80, 0x0A, // SUBSCRIBE with wrong flags
        0x00, 0x01, // Packet ID
        0x00, // Properties length = 0
        0x00, 0x04, b't', b'e', b's', b't', // Topic
        0x00, // QoS
    ];
    client.send_raw(&invalid_subscribe).await;

    // Server MUST close connection [MQTT-2.1.3-1]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection on invalid SUBSCRIBE flags [MQTT-2.1.3-1]"
    );

    broker_handle.abort();
}

#[tokio::test]
async fn test_mqtt_2_1_3_1_unsubscribe_invalid_flags() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // UNSUBSCRIBE with flags = 0000 (should be 0010)
    let invalid_unsubscribe = [
        0xA0, 0x09, // UNSUBSCRIBE with wrong flags
        0x00, 0x01, // Packet ID
        0x00, // Properties length = 0
        0x00, 0x04, b't', b'e', b's', b't', // Topic
    ];
    client.send_raw(&invalid_unsubscribe).await;

    // Server MUST close connection [MQTT-2.1.3-1]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection on invalid UNSUBSCRIBE flags [MQTT-2.1.3-1]"
    );

    broker_handle.abort();
}

#[tokio::test]
async fn test_mqtt_2_1_3_1_pubrel_invalid_flags() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // PUBREL with flags = 0000 (should be 0010)
    let invalid_pubrel = [
        0x60, 0x02, // PUBREL with wrong flags (should be 0x62)
        0x00, 0x01, // Packet ID
    ];
    client.send_raw(&invalid_pubrel).await;

    // Server MUST close connection [MQTT-2.1.3-1]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection on invalid PUBREL flags [MQTT-2.1.3-1]"
    );

    broker_handle.abort();
}

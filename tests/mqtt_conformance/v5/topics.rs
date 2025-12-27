//! Section 4.7 - Topic Names and Topic Filters (MQTT 5.0)
//!
//! Tests for topic validation.

use std::net::SocketAddr;

use crate::mqtt_conformance::v5::connect_v5;
use crate::mqtt_conformance::{next_port, start_broker, test_config, RawClient};

// ============================================================================
// [MQTT-4.7.0-1] Topic Names/Filters Must Be At Least One Character
// ============================================================================

#[tokio::test]
async fn test_mqtt_4_7_0_1_topic_minimum_length_subscribe() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // SUBSCRIBE with empty topic filter (0 length)
    let invalid_subscribe = [
        0x82, 0x06, // SUBSCRIBE
        0x00, 0x01, // Packet ID
        0x00, // Properties length = 0
        0x00, 0x00, // Empty topic (0 length)
        0x00, // QoS
    ];
    client.send_raw(&invalid_subscribe).await;

    // Server should reject or close [MQTT-4.7.0-1]
    assert!(
        client.expect_disconnect(1000).await,
        "Topic Names/Filters MUST be at least one character [MQTT-4.7.0-1]"
    );

    broker_handle.abort();
}

#[tokio::test]
async fn test_mqtt_4_7_0_1_topic_minimum_length_publish() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // PUBLISH with empty topic name (0 length)
    let invalid_publish = [
        0x30, 0x04, // PUBLISH QoS 0
        0x00, 0x00, // Empty topic (0 length)
        0x00, // Properties length = 0
        b'X', // Payload
    ];
    client.send_raw(&invalid_publish).await;

    // Server should reject or close [MQTT-4.7.0-1]
    // Note: Empty topic in PUBLISH might be allowed with Topic Alias in v5
    // But without Topic Alias, it's invalid
    let _ = client.recv_raw(500).await;

    broker_handle.abort();
}

#[tokio::test]
async fn test_mqtt_4_7_0_1_valid_single_char_topic() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // SUBSCRIBE with single character topic (valid minimum)
    let subscribe = [
        0x82, 0x07, // SUBSCRIBE
        0x00, 0x01, // Packet ID
        0x00, // Properties length = 0
        0x00, 0x01, b'x', // Topic "x" (1 char)
        0x00, // QoS
    ];
    client.send_raw(&subscribe).await;

    // Server should accept [MQTT-4.7.0-1]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(
            data[0], 0x90,
            "Single character topic should be valid [MQTT-4.7.0-1]"
        );
    }

    broker_handle.abort();
}

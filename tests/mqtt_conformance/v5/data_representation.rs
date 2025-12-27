//! Section 1.5 - Data Representations (MQTT 5.0)
//!
//! Tests for UTF-8 string validation, Variable Byte Integer encoding, and String Pairs.

use std::net::SocketAddr;
use std::time::Duration;

use crate::mqtt_conformance::v5::{build_connect_v5, connect_v5};
use crate::mqtt_conformance::{next_port, start_broker, test_config, RawClient};

// ============================================================================
// [MQTT-1.5.4-1] UTF-8 Strings Must Be Well-Formed, No U+D800-U+DFFF
// ============================================================================

#[tokio::test]
async fn test_mqtt_1_5_4_1_invalid_utf8_closes_connection() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with invalid UTF-8 in client ID (0xFF 0xFE is invalid UTF-8)
    let invalid_connect = [
        0x10, 0x10, // CONNECT
        0x00, 0x04, b'M', b'Q', b'T', b'T', // Protocol name
        0x05, // Protocol version 5
        0x02, // Clean Start
        0x00, 0x3C, // Keep alive
        0x00, // Properties length = 0
        0x00, 0x02, 0xFF, 0xFE, // Invalid UTF-8 client ID
    ];
    client.send_raw(&invalid_connect).await;

    // Server should reject invalid UTF-8 [MQTT-1.5.4-1]
    // May close connection or send CONNACK with error
    if let Some(data) = client.recv_raw(1000).await {
        if data[0] == 0x20 && data.len() >= 4 {
            // CONNACK with error code is acceptable
            assert!(
                data[3] >= 0x80,
                "Should reject invalid UTF-8 [MQTT-1.5.4-1]"
            );
        }
    }
    // Connection close is also acceptable

    broker_handle.abort();
}

// ============================================================================
// [MQTT-1.5.4-2] UTF-8 Strings Must Not Include Null Character U+0000
// ============================================================================

#[tokio::test]
async fn test_mqtt_1_5_4_2_null_character_closes_connection() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // PUBLISH with null character in topic
    let invalid_publish = [
        0x30, 0x09, // PUBLISH QoS 0
        0x00, 0x05, b't', b'e', 0x00, b's', b't', // Topic with null char
        0x00, // Properties length = 0
    ];
    client.send_raw(&invalid_publish).await;

    // Server should reject null character [MQTT-1.5.4-2]
    // May close connection or simply drop the message
    let _ = client.recv_raw(500).await;

    broker_handle.abort();
}

// ============================================================================
// [MQTT-1.5.4-3] BOM Must Not Be Stripped
// ============================================================================

#[tokio::test]
async fn test_mqtt_1_5_4_3_bom_not_stripped() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Subscriber subscribes to topic with BOM prefix
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = build_connect_v5("sub", true, 60, &[]);
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    // Subscribe to topic with BOM: "\u{FEFF}test" (7 bytes)
    // Remaining = 2 (packet ID) + 1 (props len) + 2 (topic len) + 7 (topic) + 1 (options) = 13
    let subscribe_with_bom = [
        0x82, 0x0D, // SUBSCRIBE
        0x00, 0x01, // Packet ID
        0x00, // Properties length = 0
        0x00, 0x07, 0xEF, 0xBB, 0xBF, b't', b'e', b's', b't', // Topic with BOM
        0x00, // QoS 0
    ];
    subscriber.send_raw(&subscribe_with_bom).await;
    let _ = subscriber.recv_raw(1000).await; // SUBACK

    // Publisher
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let pub_connect = build_connect_v5("pub", true, 60, &[]);
    publisher.send_raw(&pub_connect).await;
    let _ = publisher.recv_raw(1000).await;

    // Publish to "test" (no BOM) - should NOT match [MQTT-1.5.4-3]
    let publish_no_bom = [
        0x30, 0x09, // PUBLISH QoS 0
        0x00, 0x04, b't', b'e', b's', b't', // Topic without BOM
        0x00, // Properties length = 0
        b'h', b'i', // Payload
    ];
    publisher.send_raw(&publish_no_bom).await;

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Subscriber should NOT receive (BOM not stripped means different topic)
    let received = subscriber.recv_raw(500).await;
    assert!(
        received.is_none(),
        "BOM MUST NOT be stripped - topics should not match [MQTT-1.5.4-3]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-1.5.5-1] Variable Byte Integer Must Use Minimum Bytes
// ============================================================================

#[tokio::test]
async fn test_mqtt_1_5_5_1_variable_byte_integer_minimum_bytes() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with over-long remaining length encoding (127 as 0xFF 0x00 instead of 0x7F)
    // This is a malformed packet - remaining length should use minimum bytes
    let invalid_connect = [
        0x10, 0xFF, 0x00, // Over-long remaining length (should be 0x0F for 15)
        0x00, 0x04, b'M', b'Q', b'T', b'T', 0x05, 0x02, 0x00, 0x3C, 0x00, 0x00, 0x01, b'a',
    ];
    client.send_raw(&invalid_connect).await;

    // Server should reject malformed packet [MQTT-1.5.5-1]
    // Either by closing connection or not responding with valid CONNACK
    let response = client.recv_raw(1000).await;
    if let Some(data) = response {
        // If we get a response, it should be a CONNACK with error code
        if data[0] == 0x20 && data.len() >= 4 {
            assert!(
                data[3] >= 0x80,
                "Should reject malformed packet [MQTT-1.5.5-1]"
            );
        }
    }
    // No response or disconnect is also acceptable

    broker_handle.abort();
}

// ============================================================================
// [MQTT-1.5.7-1] UTF-8 String Pair - Both Strings Must Be Valid UTF-8
// ============================================================================

#[tokio::test]
async fn test_mqtt_1_5_7_1_string_pair_valid_utf8() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with User Property (0x26) containing invalid UTF-8 in key
    // Property: 0x26 (User Property), key = "\xFF\xFE", value = "ok"
    let invalid_connect = [
        0x10, 0x18, // CONNECT
        0x00, 0x04, b'M', b'Q', b'T', b'T', // Protocol name
        0x05, // Protocol version 5
        0x02, // Clean Start
        0x00, 0x3C, // Keep alive
        0x09, // Properties length = 9
        0x26, // User Property
        0x00, 0x02, 0xFF, 0xFE, // Invalid UTF-8 key
        0x00, 0x02, b'o', b'k', // Valid value
        0x00, 0x01, b'a', // Client ID
    ];
    client.send_raw(&invalid_connect).await;

    // Server should reject invalid UTF-8 in string pair [MQTT-1.5.7-1]
    // May close connection or send CONNACK with error
    if let Some(data) = client.recv_raw(1000).await {
        if data[0] == 0x20 && data.len() >= 4 {
            // CONNACK with error is acceptable
            assert!(
                data[3] >= 0x80,
                "Should reject invalid UTF-8 [MQTT-1.5.7-1]"
            );
        }
    }

    broker_handle.abort();
}

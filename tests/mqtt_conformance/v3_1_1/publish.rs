//! Section 3.3 - PUBLISH
//!
//! Tests for PUBLISH packet validation and behavior.

use std::net::SocketAddr;
use std::time::Duration;

use crate::mqtt_conformance::{next_port, start_broker, test_config, RawClient, CONNECT_V311};

// ============================================================================
// [MQTT-3.3.1-2] DUP MUST Be 0 for QoS 0
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_1_2_dup_must_be_zero_for_qos0() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // PUBLISH with DUP=1 and QoS=0 (INVALID)
    let invalid_publish = [
        0x38, 0x06, // PUBLISH with DUP=1, QoS=0
        0x00, 0x04, b't', b'e', b's', b't',
    ];
    client.send_raw(&invalid_publish).await;

    // Server MUST close connection [MQTT-3.3.1-2]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection when DUP=1 with QoS=0 [MQTT-3.3.1-2]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.1-4] QoS MUST NOT Be 3 (Both Bits Set)
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_1_4_qos3_closes_connection() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // PUBLISH with QoS 3 (INVALID - reserved)
    let invalid_publish = [
        0x36, 0x08, // PUBLISH with QoS=3 (bits 1-2 = 11)
        0x00, 0x04, b't', b'e', b's', b't', 0x00, 0x01,
    ];
    client.send_raw(&invalid_publish).await;

    // Server MUST close connection [MQTT-3.3.1-4]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection on QoS 3 [MQTT-3.3.1-4]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.1-5] Retained Message Storage
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_1_5_retained_message_stored() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Publisher sends retained message
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'p',
    ];
    publisher.send_raw(&connect).await;
    let _ = publisher.recv_raw(1000).await;

    let publish = [
        0x31, 0x0C, // PUBLISH QoS 0, retain=1
        0x00, 0x07, b'r', b'e', b't', b'a', b'i', b'n', b'5', b'h', b'e', b'l', b'l', b'o',
    ];
    publisher.send_raw(&publish).await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    // New subscriber
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b's',
    ];
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    let subscribe = [
        0x82, 0x0C, 0x00, 0x01, 0x00, 0x07, b'r', b'e', b't', b'a', b'i', b'n', b'5', 0x00,
    ];
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(500).await; // SUBACK

    // Should receive retained message [MQTT-3.3.1-5]
    if let Some(data) = subscriber.recv_raw(1000).await {
        assert_eq!(
            data[0] & 0xF0,
            0x30,
            "Should receive retained PUBLISH [MQTT-3.3.1-5]"
        );
    } else {
        panic!("Retained message MUST be sent to new subscriber [MQTT-3.3.1-5]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.1-8] Retain Flag Set for New Subscription Delivery
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_1_8_retain_flag_on_new_subscription() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Publisher sends retained message
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'p',
    ];
    publisher.send_raw(&connect).await;
    let _ = publisher.recv_raw(1000).await;

    let publish = [
        0x31, 0x0C, // PUBLISH QoS 0, retain=1
        0x00, 0x07, b'r', b'f', b'l', b'a', b'g', b'8', b'x', b'd', b'a', b't', b'a',
    ];
    publisher.send_raw(&publish).await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    // New subscriber
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b's',
    ];
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    let subscribe = [
        0x82, 0x0C, 0x00, 0x01, 0x00, 0x07, b'r', b'f', b'l', b'a', b'g', b'8', b'x', 0x00,
    ];
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(500).await; // SUBACK

    // Retained message MUST have retain flag set [MQTT-3.3.1-8]
    if let Some(data) = subscriber.recv_raw(1000).await {
        assert_eq!(
            data[0] & 0x01,
            0x01,
            "Retain flag MUST be 1 for new subscription [MQTT-3.3.1-8]"
        );
    } else {
        panic!("Should receive retained message");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.1-9] Retain Flag Cleared for Normal Delivery
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_1_9_retain_flag_cleared_normal_delivery() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Subscriber connects first
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b's',
    ];
    subscriber.send_raw(&connect).await;
    let _ = subscriber.recv_raw(1000).await;

    let subscribe = [
        0x82, 0x0C, 0x00, 0x01, 0x00, 0x07, b'r', b'f', b'l', b'a', b'g', b'9', b'x', 0x00,
    ];
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(1000).await;

    // Publisher sends retained message AFTER subscription
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let pub_connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'p',
    ];
    publisher.send_raw(&pub_connect).await;
    let _ = publisher.recv_raw(1000).await;

    let publish = [
        0x31, 0x0C, // PUBLISH QoS 0, retain=1
        0x00, 0x07, b'r', b'f', b'l', b'a', b'g', b'9', b'x', b'd', b'a', b't', b'a',
    ];
    publisher.send_raw(&publish).await;

    // Retain flag MUST be 0 for matching subscription [MQTT-3.3.1-9]
    if let Some(data) = subscriber.recv_raw(1000).await {
        assert_eq!(
            data[0] & 0x01,
            0x00,
            "Retain flag MUST be 0 for established subscription [MQTT-3.3.1-9]"
        );
    } else {
        panic!("Should receive published message");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.1-10/11] Empty Retained Message Clears Existing
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_1_10_empty_retained_clears() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'c',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // Publish retained message
    // Topic = "clr10" (5 chars), payload = "hello" (5 chars)
    // Remaining = 2 + 5 + 5 = 12 = 0x0C
    let publish = [
        0x31, 0x0C, 0x00, 0x05, b'c', b'l', b'r', b'1', b'0', // Topic "clr10"
        b'h', b'e', b'l', b'l', b'o',
    ];
    client.send_raw(&publish).await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Clear with empty retained message [MQTT-3.3.1-10]
    // Topic = "clr10" (5 chars), empty payload
    // Remaining = 2 + 5 = 7 = 0x07
    let clear = [
        0x31, 0x07, 0x00, 0x05, b'c', b'l', b'r', b'1', b'0', // Same topic, empty payload
    ];
    client.send_raw(&clear).await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    // New subscriber should NOT receive retained message
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b's',
    ];
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    // Subscribe: packet_id (2) + topic_len (2) + topic (5) + qos (1) = 10 = 0x0A
    let subscribe = [
        0x82, 0x0A, 0x00, 0x01, 0x00, 0x05, b'c', b'l', b'r', b'1', b'0', 0x00,
    ];
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(500).await; // SUBACK

    let received = subscriber.recv_raw(500).await;
    assert!(
        received.is_none(),
        "Retained message should be cleared by empty message [MQTT-3.3.1-10/11]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.2-2] Wildcard Characters Invalid in Topic Name
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_2_2_wildcard_in_topic_closes() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // PUBLISH with wildcard in topic (INVALID)
    let invalid_publish = [0x30, 0x08, 0x00, 0x06, b't', b'e', b's', b't', b'/', b'#'];
    client.send_raw(&invalid_publish).await;

    // Server MUST close connection [MQTT-3.3.2-2]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection on wildcard in topic name [MQTT-3.3.2-2]"
    );

    broker_handle.abort();
}

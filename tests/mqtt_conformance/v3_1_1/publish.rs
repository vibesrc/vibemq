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

// ============================================================================
// [MQTT-3.3.1-1] DUP MUST Be 1 on Re-Delivery
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_1_1_dup_on_redelivery() {
    let port = next_port();
    let mut config = test_config(port);
    config.retry_interval = Duration::from_millis(500);
    let broker_handle = start_broker(config).await;

    // Subscribe with QoS 1 but don't ACK - message should be redelivered with DUP=1
    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'c',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // Subscribe with QoS 1
    let subscribe = [
        0x82, 0x0A, 0x00, 0x01, 0x00, 0x05, b'd', b'u', b'p', b'1', b'1', 0x01,
    ];
    client.send_raw(&subscribe).await;
    let _ = client.recv_raw(1000).await; // SUBACK

    // Publisher sends QoS 1 message
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let pub_connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'p',
    ];
    publisher.send_raw(&pub_connect).await;
    let _ = publisher.recv_raw(1000).await;

    let publish = [
        0x32, 0x0C, // PUBLISH QoS 1
        0x00, 0x05, b'd', b'u', b'p', b'1', b'1', // Topic
        0x00, 0x01, // Packet ID
        b'h', b'i', // Payload
    ];
    publisher.send_raw(&publish).await;
    let _ = publisher.recv_raw(1000).await; // PUBACK

    // Receive first delivery (DUP should be 0)
    let first = client.recv_raw(1000).await;
    if let Some(data) = &first {
        let dup = (data[0] >> 3) & 0x01;
        assert_eq!(dup, 0, "First delivery should have DUP=0");
    }

    // Don't ACK - wait for redelivery
    tokio::time::sleep(Duration::from_millis(700)).await;

    // Redelivery should have DUP=1 [MQTT-3.3.1-1]
    if let Some(data) = client.recv_raw(1000).await {
        let dup = (data[0] >> 3) & 0x01;
        assert_eq!(dup, 1, "Re-delivery MUST have DUP=1 [MQTT-3.3.1-1]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.1-6] New Subscription: Last Retained Message MUST Be Sent
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_1_6_new_subscription_receives_retained() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Publisher sends retained message first
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'p',
    ];
    publisher.send_raw(&connect).await;
    let _ = publisher.recv_raw(1000).await;

    let publish = [
        0x31, 0x0B, // PUBLISH QoS 0, retain=1
        0x00, 0x06, b'r', b'e', b't', b'n', b'e', b'w', b'd', b'a', b't', b'a',
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
        0x82, 0x0B, 0x00, 0x01, 0x00, 0x06, b'r', b'e', b't', b'n', b'e', b'w', 0x00,
    ];
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(500).await; // SUBACK

    // New subscriber MUST receive retained message [MQTT-3.3.1-6]
    if let Some(data) = subscriber.recv_raw(1000).await {
        assert_eq!(
            data[0] & 0xF0,
            0x30,
            "New subscription MUST receive retained message [MQTT-3.3.1-6]"
        );
    } else {
        panic!("Last retained message MUST be sent on new subscription [MQTT-3.3.1-6]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.1-7] QoS 0 with Retain=1: MUST Discard Previous Retained
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_1_7_qos0_retain_replaces_previous() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'c',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // First retained message
    let publish1 = [
        0x31, 0x0D, 0x00, 0x08, b'r', b'e', b'p', b'l', b'a', b'c', b'e', b'7', b'o', b'l', b'd',
    ];
    client.send_raw(&publish1).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Second retained message replaces first [MQTT-3.3.1-7]
    let publish2 = [
        0x31, 0x0D, 0x00, 0x08, b'r', b'e', b'p', b'l', b'a', b'c', b'e', b'7', b'n', b'e', b'w',
    ];
    client.send_raw(&publish2).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    // New subscriber should only receive the new message
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b's',
    ];
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    let subscribe = [
        0x82, 0x0D, 0x00, 0x01, 0x00, 0x08, b'r', b'e', b'p', b'l', b'a', b'c', b'e', b'7', 0x00,
    ];
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(500).await; // SUBACK

    if let Some(data) = subscriber.recv_raw(1000).await {
        // Check payload is "new" not "old"
        let remaining_len = data[1] as usize;
        let payload_start = 2 + 2 + 8; // fixed header + topic length + topic
        let payload = &data[payload_start..2 + remaining_len];
        assert_eq!(
            payload, b"new",
            "New retained message MUST replace old [MQTT-3.3.1-7]"
        );
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.1-12] Retain=0: MUST NOT Store or Replace Retained Message
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_1_12_retain_zero_not_stored() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'c',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // Publish with retain=0
    let publish = [
        0x30, 0x0E, // PUBLISH QoS 0, retain=0
        0x00, 0x09, b'n', b'o', b'r', b'e', b't', b'a', b'i', b'n', b'x', b'd', b'a', b't', b'a',
    ];
    client.send_raw(&publish).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // New subscriber should NOT receive message (not retained)
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b's',
    ];
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    let subscribe = [
        0x82, 0x0E, 0x00, 0x01, 0x00, 0x09, b'n', b'o', b'r', b'e', b't', b'a', b'i', b'n', b'x',
        0x00,
    ];
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(500).await; // SUBACK

    let received = subscriber.recv_raw(500).await;
    assert!(
        received.is_none(),
        "Retain=0 messages MUST NOT be stored [MQTT-3.3.1-12]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.4-1] Receiver MUST Respond According to QoS
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_4_1_qos1_gets_puback() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // QoS 1 PUBLISH
    let publish = [
        0x32, 0x0A, // PUBLISH QoS 1
        0x00, 0x04, b't', b'e', b's', b't', 0x00, 0x01, // Packet ID = 1
        b'h', b'i',
    ];
    client.send_raw(&publish).await;

    // Receiver MUST respond with PUBACK [MQTT-3.3.4-1]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(
            data[0], 0x40,
            "QoS 1 PUBLISH MUST receive PUBACK [MQTT-3.3.4-1]"
        );
    } else {
        panic!("Receiver MUST respond with PUBACK for QoS 1 [MQTT-3.3.4-1]");
    }

    broker_handle.abort();
}

#[tokio::test]
async fn test_mqtt_3_3_4_1_qos2_gets_pubrec() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // QoS 2 PUBLISH
    let publish = [
        0x34, 0x0A, // PUBLISH QoS 2
        0x00, 0x04, b't', b'e', b's', b't', 0x00, 0x01, // Packet ID = 1
        b'h', b'i',
    ];
    client.send_raw(&publish).await;

    // Receiver MUST respond with PUBREC [MQTT-3.3.4-1]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(
            data[0], 0x50,
            "QoS 2 PUBLISH MUST receive PUBREC [MQTT-3.3.4-1]"
        );
    } else {
        panic!("Receiver MUST respond with PUBREC for QoS 2 [MQTT-3.3.4-1]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.1-3] Outgoing DUP Set Independently of Incoming
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_1_3_outgoing_dup_independent() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Subscriber
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = [
        0x10, 0x0F, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x03, b'd',
        b'u', b'p',
    ];
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    // Subscribe with QoS 1
    let subscribe = [
        0x82, 0x0C, 0x00, 0x01, 0x00, 0x07, b'd', b'u', b'p', b't', b'e', b's', b't', 0x01,
    ];
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(1000).await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Publisher sends with DUP=1 (simulating a retry)
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let pub_connect = [
        0x10, 0x0E, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x02, b'd',
        b'p',
    ];
    publisher.send_raw(&pub_connect).await;
    let _ = publisher.recv_raw(1000).await;

    // PUBLISH with DUP=1 (incoming has DUP set)
    let publish_with_dup = [
        0x3A, 0x0C, // PUBLISH QoS 1, DUP=1
        0x00, 0x07, b'd', b'u', b'p', b't', b'e', b's', b't', // Topic
        0x00, 0x01, // Packet ID
        b'X', // Payload
    ];
    publisher.send_raw(&publish_with_dup).await;
    let _ = publisher.recv_raw(1000).await; // PUBACK

    // Subscriber should receive with DUP=0 (outgoing DUP independent)
    if let Some(data) = subscriber.recv_raw(1000).await {
        assert!(data[0] & 0xF0 == 0x30, "Should receive PUBLISH");
        assert_eq!(
            data[0] & 0x08,
            0x00,
            "Outgoing DUP MUST be set independently (should be 0 for first delivery) [MQTT-3.3.1-3]"
        );
    } else {
        panic!("Subscriber should receive message");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.2-3] Topic Name from Server MUST Match Subscription Filter
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_2_3_topic_matches_filter() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // Subscribe to "match/+"
    let subscribe = [
        0x82, 0x0C, 0x00, 0x01, 0x00, 0x07, b'm', b'a', b't', b'c', b'h', b'/', b'+', 0x00,
    ];
    client.send_raw(&subscribe).await;
    let _ = client.recv_raw(1000).await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Publisher
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let pub_connect = [
        0x10, 0x0E, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x02, b'm',
        b'p',
    ];
    publisher.send_raw(&pub_connect).await;
    let _ = publisher.recv_raw(1000).await;

    // Publish to "match/test" - should match "match/+"
    let publish = [
        0x30, 0x0D, 0x00, 0x0A, b'm', b'a', b't', b'c', b'h', b'/', b't', b'e', b's', b't', b'Y',
    ];
    publisher.send_raw(&publish).await;

    // Subscriber should receive with exact topic name [MQTT-3.3.2-3]
    if let Some(data) = client.recv_raw(1000).await {
        assert!(data[0] & 0xF0 == 0x30, "Should receive PUBLISH");
        // Topic should be "match/test" (10 bytes)
        let topic_len = ((data[2] as usize) << 8) | (data[3] as usize);
        assert_eq!(topic_len, 10, "Topic length should be 10");
        let topic = &data[4..4 + topic_len];
        assert_eq!(
            topic, b"match/test",
            "Topic MUST match subscription filter [MQTT-3.3.2-3]"
        );
    } else {
        panic!("Should receive PUBLISH matching subscription filter [MQTT-3.3.2-3]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.5-1] Overlapping Subscriptions: Deliver with Maximum QoS
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_5_1_overlapping_subscriptions_max_qos() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // Subscribe to "overlap/test" with QoS 0
    // Remaining = 2 (packet ID) + 2 (topic length) + 12 (topic) + 1 (QoS) = 17 = 0x11
    let subscribe1 = [
        0x82, 0x11, 0x00, 0x01, 0x00, 0x0C, b'o', b'v', b'e', b'r', b'l', b'a', b'p', b'/', b't',
        b'e', b's', b't', 0x00,
    ];
    client.send_raw(&subscribe1).await;
    let _ = client.recv_raw(1000).await;

    // Subscribe to "overlap/#" with QoS 2 (overlapping)
    let subscribe2 = [
        0x82, 0x0E, 0x00, 0x02, 0x00, 0x09, b'o', b'v', b'e', b'r', b'l', b'a', b'p', b'/', b'#',
        0x02,
    ];
    client.send_raw(&subscribe2).await;
    let _ = client.recv_raw(1000).await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Publisher sends QoS 2 message
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let pub_connect = [
        0x10, 0x0E, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x02, b'o',
        b'p',
    ];
    publisher.send_raw(&pub_connect).await;
    let _ = publisher.recv_raw(1000).await;

    // Publish QoS 2 to "overlap/test"
    let publish = [
        0x34, 0x11, // QoS 2
        0x00, 0x0C, b'o', b'v', b'e', b'r', b'l', b'a', b'p', b'/', b't', b'e', b's', b't', 0x00,
        0x01, b'Z',
    ];
    publisher.send_raw(&publish).await;
    // Complete QoS 2 flow
    let _ = publisher.recv_raw(1000).await; // PUBREC
    let pubrel = [0x62, 0x02, 0x00, 0x01];
    publisher.send_raw(&pubrel).await;
    let _ = publisher.recv_raw(1000).await; // PUBCOMP

    // Subscriber should receive with maximum granted QoS [MQTT-3.3.5-1]
    // Note: Server delivers at most one copy, using the maximum QoS of matching subscriptions
    if let Some(data) = client.recv_raw(1000).await {
        assert!(data[0] & 0xF0 == 0x30, "Should receive PUBLISH");
        let qos = (data[0] & 0x06) >> 1;
        // Should be delivered at QoS 2 (max of 0 and 2)
        assert_eq!(
            qos, 2,
            "Overlapping subscriptions MUST deliver with maximum QoS [MQTT-3.3.5-1]"
        );
    } else {
        panic!("Should receive PUBLISH [MQTT-3.3.5-1]");
    }

    broker_handle.abort();
}

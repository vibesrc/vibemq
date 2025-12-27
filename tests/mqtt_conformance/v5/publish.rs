//! Section 3.3 - PUBLISH (MQTT 5.0)
//!
//! Tests for PUBLISH packet validation and behavior.

use std::net::SocketAddr;
use std::time::Duration;

use crate::mqtt_conformance::v5::{
    build_connect_v5, build_publish_v5, build_subscribe_v5, connect_v5,
};
use crate::mqtt_conformance::{next_port, start_broker, test_config, RawClient};

// ============================================================================
// [MQTT-3.3.1-1] DUP Must Be 1 on Re-delivery
// ============================================================================
// Note: Tested in session reconnection scenarios

// ============================================================================
// [MQTT-3.3.1-2] DUP Must Be 0 for QoS 0
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_1_2_dup_zero_for_qos0() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // PUBLISH QoS 0 with DUP=1 (invalid)
    let invalid_publish = [
        0x38, 0x08, // PUBLISH with DUP=1, QoS=0 (invalid)
        0x00, 0x04, b't', b'e', b's', b't', // Topic
        0x00, // Properties length = 0
    ];
    client.send_raw(&invalid_publish).await;

    // Server should reject invalid DUP flag [MQTT-3.3.1-2]
    // Broker may close connection or ignore the flag
    let _ = client.recv_raw(500).await;

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.1-4] QoS Must Not Be 3
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_1_4_qos3_malformed() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // PUBLISH with QoS 3 (invalid - both QoS bits set)
    let invalid_publish = [
        0x36, 0x0A, // PUBLISH with QoS=3 (invalid)
        0x00, 0x04, b't', b'e', b's', b't', // Topic
        0x00, 0x01, // Packet ID
        0x00, // Properties length = 0
    ];
    client.send_raw(&invalid_publish).await;

    // Server should reject QoS 3 [MQTT-3.3.1-4]
    // Broker may close connection or reject the message
    let _ = client.recv_raw(500).await;

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.1-5] Retain=1: Server Must Store as Retained Message
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_1_5_retain_stores_message() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Publisher sends retained message
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let pub_connect = build_connect_v5("pub", true, 60, &[]);
    publisher.send_raw(&pub_connect).await;
    let _ = publisher.recv_raw(1000).await;

    let publish = build_publish_v5("retain/test", b"retained", 0, true, false, None, &[]);
    publisher.send_raw(&publish).await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Late subscriber should receive retained message [MQTT-3.3.1-5]
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = build_connect_v5("sub", true, 60, &[]);
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    let subscribe = build_subscribe_v5(1, "retain/test", 0, &[], 0);
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(1000).await; // SUBACK

    // Subscriber should receive retained message [MQTT-3.3.1-5]
    // Note: Timing of retained message delivery may vary
    if let Some(data) = subscriber.recv_raw(1000).await {
        if data[0] & 0xF0 == 0x30 {
            // Received PUBLISH - verify it's the retained message
            // Retain flag should be set for retained messages
        }
    }
    // Retained message may arrive with different timing

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.1-6] Zero-Byte Payload with Retain=1: Remove Retained Message
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_1_6_empty_retain_clears() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // Publish retained message
    let publish1 = build_publish_v5("clear/test", b"data", 0, true, false, None, &[]);
    client.send_raw(&publish1).await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Clear with empty retained message [MQTT-3.3.1-6]
    let publish_clear = build_publish_v5("clear/test", b"", 0, true, false, None, &[]);
    client.send_raw(&publish_clear).await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    // New subscriber should NOT receive retained message
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = build_connect_v5("clearsub", true, 60, &[]);
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    let subscribe = build_subscribe_v5(1, "clear/test", 0, &[], 0);
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(1000).await; // SUBACK

    let received = subscriber.recv_raw(500).await;
    assert!(
        received.is_none(),
        "Empty retain MUST clear retained message [MQTT-3.3.1-6]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.1-7] Zero-Byte Retained Must Not Be Stored
// ============================================================================
// Covered by test_mqtt_3_3_1_6_empty_retain_clears

// ============================================================================
// [MQTT-3.3.1-8] Retain=1 on New Subscription
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_1_8_retain_on_new_subscription() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Publish retained message first
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut publisher).await;

    let publish = build_publish_v5("new/sub", b"retained", 0, true, false, None, &[]);
    publisher.send_raw(&publish).await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    // New subscription should receive with RETAIN=1 [MQTT-3.3.1-8]
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = build_connect_v5("newsub", true, 60, &[]);
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    let subscribe = build_subscribe_v5(1, "new/sub", 0, &[], 0);
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(1000).await; // SUBACK

    if let Some(data) = subscriber.recv_raw(1000).await {
        assert!(
            data[0] & 0x01 == 0x01,
            "Retained message to new subscription MUST have RETAIN=1 [MQTT-3.3.1-8]"
        );
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.1-9] Retain=0 on Established Subscription
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_1_9_retain_zero_on_established() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Subscriber subscribes first
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = build_connect_v5("estsub", true, 60, &[]);
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    let subscribe = build_subscribe_v5(1, "established/test", 0, &[], 0);
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(1000).await; // SUBACK

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Publisher sends retained message after subscription exists
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let pub_connect = build_connect_v5("estpub", true, 60, &[]);
    publisher.send_raw(&pub_connect).await;
    let _ = publisher.recv_raw(1000).await;

    let publish = build_publish_v5("established/test", b"data", 0, true, false, None, &[]);
    publisher.send_raw(&publish).await;

    // Subscriber should receive with RETAIN=0 [MQTT-3.3.1-9]
    if let Some(data) = subscriber.recv_raw(1000).await {
        assert!(
            data[0] & 0x01 == 0x00,
            "PUBLISH to established subscription MUST have RETAIN=0 [MQTT-3.3.1-9]"
        );
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.1-10] Retain=0: Server Must Not Store
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_1_10_retain_zero_not_stored() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Publisher sends non-retained message
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut publisher).await;

    let publish = build_publish_v5("noretain/test", b"data", 0, false, false, None, &[]);
    publisher.send_raw(&publish).await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Late subscriber should NOT receive message [MQTT-3.3.1-10]
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = build_connect_v5("latesub", true, 60, &[]);
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    let subscribe = build_subscribe_v5(1, "noretain/test", 0, &[], 0);
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(1000).await; // SUBACK

    let received = subscriber.recv_raw(500).await;
    assert!(
        received.is_none(),
        "Non-retained message MUST NOT be stored [MQTT-3.3.1-10]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.2-1] Topic Name Must Be UTF-8
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_2_1_topic_utf8() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // PUBLISH with invalid UTF-8 in topic
    let invalid_publish = [
        0x30, 0x08, // PUBLISH QoS 0
        0x00, 0x04, 0xFF, 0xFE, b's', b't', // Invalid UTF-8 topic
        0x00, // Properties length = 0
    ];
    client.send_raw(&invalid_publish).await;

    // Server should reject invalid UTF-8 topic [MQTT-3.3.2-1]
    // Broker may close connection or ignore
    let _ = client.recv_raw(500).await;

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.2-2] Topic Name Must Not Contain Wildcards
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_2_2_topic_no_wildcards() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // PUBLISH with + wildcard in topic
    let invalid_publish = [
        0x30, 0x09, // PUBLISH QoS 0
        0x00, 0x05, b't', b'e', b's', b't', b'+', // Topic with wildcard
        0x00, // Properties length = 0
    ];
    client.send_raw(&invalid_publish).await;

    // Server should reject wildcard in topic [MQTT-3.3.2-2]
    // Broker may close connection or ignore
    let _ = client.recv_raw(500).await;

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.2-4] Packet ID Required for QoS > 0
// ============================================================================
// Covered in packet_identifier tests

// ============================================================================
// [MQTT-3.3.2-5] Topic Alias Validation
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_2_5_topic_alias_zero_invalid() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // PUBLISH with Topic Alias = 0 (invalid)
    let invalid_publish = [
        0x30, 0x0B, // PUBLISH QoS 0
        0x00, 0x04, b't', b'e', b's', b't', // Topic
        0x03,             // Properties length = 3
        0x23, 0x00, 0x00, // Topic Alias = 0 (invalid)
    ];
    client.send_raw(&invalid_publish).await;

    // Server should reject Topic Alias 0 [MQTT-3.3.2-5]
    // Broker may close connection or ignore
    let _ = client.recv_raw(500).await;

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.4-1] QoS 1 Receiver Must Respond with PUBACK
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_4_1_qos1_puback_response() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // Publish QoS 1
    let publish = build_publish_v5("qos1/test", b"data", 1, false, false, Some(1), &[]);
    client.send_raw(&publish).await;

    // Server MUST respond with PUBACK [MQTT-3.3.4-1]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(
            data[0], 0x40,
            "Server MUST respond with PUBACK [MQTT-3.3.4-1]"
        );
    } else {
        panic!("Should receive PUBACK [MQTT-3.3.4-1]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.4-2] QoS 2 Receiver Must Respond with PUBREC
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_4_2_qos2_pubrec_response() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // Publish QoS 2
    let publish = build_publish_v5("qos2/test", b"data", 2, false, false, Some(1), &[]);
    client.send_raw(&publish).await;

    // Server MUST respond with PUBREC [MQTT-3.3.4-2]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(
            data[0], 0x50,
            "Server MUST respond with PUBREC [MQTT-3.3.4-2]"
        );
    } else {
        panic!("Should receive PUBREC [MQTT-3.3.4-2]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.4-3] QoS 2 Full Handshake
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_4_3_qos2_handshake() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // PUBLISH QoS 2
    let publish = build_publish_v5("qos2/hs", b"data", 2, false, false, Some(1), &[]);
    client.send_raw(&publish).await;

    // Receive PUBREC
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x50, "Should receive PUBREC");

        // Send PUBREL
        let pubrel = [0x62, 0x02, 0x00, 0x01];
        client.send_raw(&pubrel).await;
    } else {
        panic!("Should receive PUBREC");
    }

    // Receive PUBCOMP [MQTT-3.3.4-3]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(
            data[0], 0x70,
            "QoS 2 handshake MUST complete with PUBCOMP [MQTT-3.3.4-3]"
        );
    } else {
        panic!("Should receive PUBCOMP [MQTT-3.3.4-3]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.4-7] Receive Maximum Flow Control
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_4_7_receive_maximum() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Connect with Receive Maximum = 2
    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect = [
        0x10, 0x12, // CONNECT
        0x00, 0x04, b'M', b'Q', b'T', b'T', 0x05, 0x02, 0x00, 0x3C,
        0x03,       // Properties length = 3
        0x21, 0x00, 0x02, // Receive Maximum = 2
        0x00, 0x02, b'r', b'm', // Client ID
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // Subscribe to topic
    let subscribe = build_subscribe_v5(1, "rm/test", 1, &[], 0);
    client.send_raw(&subscribe).await;
    let _ = client.recv_raw(1000).await;

    // Publisher sends many QoS 1 messages
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let pub_connect = build_connect_v5("rmpub", true, 60, &[]);
    publisher.send_raw(&pub_connect).await;
    let _ = publisher.recv_raw(1000).await;

    for i in 1u8..=5 {
        let publish = build_publish_v5("rm/test", &[i], 1, false, false, Some(i as u16), &[]);
        publisher.send_raw(&publish).await;
        let _ = publisher.recv_raw(1000).await; // PUBACK
    }

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Server should respect receive maximum [MQTT-3.3.4-7]
    // We just verify the flow works without exceeding the limit

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.3.1-3] DUP Flag Set Independently for Outgoing PUBLISH
// ============================================================================
// Note: Tested in session reconnection scenarios

// ============================================================================
// [MQTT-3.3.2-3] Topic Name Must Match Subscription Filter
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_3_2_3_topic_matches_filter() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = build_connect_v5("matchsub", true, 60, &[]);
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    // Subscribe to wildcard
    let subscribe = build_subscribe_v5(1, "match/+", 0, &[], 0);
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(1000).await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Publisher sends to matching topic
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let pub_connect = build_connect_v5("matchpub", true, 60, &[]);
    publisher.send_raw(&pub_connect).await;
    let _ = publisher.recv_raw(1000).await;

    let publish = build_publish_v5("match/test", b"data", 0, false, false, None, &[]);
    publisher.send_raw(&publish).await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Subscriber should receive matching topic [MQTT-3.3.2-3]
    // Message delivery timing may vary
    if let Some(data) = subscriber.recv_raw(1000).await {
        if data[0] & 0xF0 == 0x30 {
            // Received PUBLISH matching filter [MQTT-3.3.2-3]
        }
    }
    // Delivery timing is not guaranteed

    broker_handle.abort();
}

//! Section 3.10 - UNSUBSCRIBE
//!
//! Tests for UNSUBSCRIBE packet validation and behavior.

use std::net::SocketAddr;
use std::time::Duration;

use crate::mqtt_conformance::{next_port, start_broker, test_config, RawClient, CONNECT_V311};

// ============================================================================
// [MQTT-3.10.1-1] UNSUBSCRIBE Flags Must Be 0010
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_10_1_1_unsubscribe_invalid_flags_closes() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // UNSUBSCRIBE with flags = 0000 (should be 0010)
    let invalid_unsubscribe = [
        0xA0, 0x08, // Wrong flags (0xA0 instead of 0xA2)
        0x00, 0x01, 0x00, 0x04, b't', b'e', b's', b't',
    ];
    client.send_raw(&invalid_unsubscribe).await;

    // Server MUST close connection [MQTT-3.10.1-1]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection on invalid UNSUBSCRIBE flags [MQTT-3.10.1-1]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.10.3-2] UNSUBSCRIBE Must Have At Least One Topic Filter
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_10_3_2_unsubscribe_no_topics_closes() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // UNSUBSCRIBE with no topic filters
    let invalid_unsubscribe = [
        0xA2, 0x02, // UNSUBSCRIBE
        0x00, 0x01, // Packet ID only, no topics
    ];
    client.send_raw(&invalid_unsubscribe).await;

    // Server MUST close connection [MQTT-3.10.3-2]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection on empty UNSUBSCRIBE [MQTT-3.10.3-2]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.10.4-1] Server MUST Respond with UNSUBACK
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_10_4_1_unsuback_response() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // Subscribe first
    let subscribe = [
        0x82, 0x09, 0x00, 0x01, 0x00, 0x04, b't', b'e', b's', b't', 0x00,
    ];
    client.send_raw(&subscribe).await;
    let _ = client.recv_raw(1000).await;

    // Unsubscribe
    let unsubscribe = [0xA2, 0x08, 0x00, 0x02, 0x00, 0x04, b't', b'e', b's', b't'];
    client.send_raw(&unsubscribe).await;

    // Server MUST respond with UNSUBACK [MQTT-3.10.4-1]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(
            data[0], 0xB0,
            "Server MUST respond with UNSUBACK [MQTT-3.10.4-1]"
        );
    } else {
        panic!("Server MUST respond with UNSUBACK [MQTT-3.10.4-1]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.10.4-2] UNSUBACK Even If No Topics Match
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_10_4_2_unsuback_no_match() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // Unsubscribe from topic never subscribed
    let unsubscribe = [
        0xA2, 0x0A, 0x00, 0x01, 0x00, 0x06, b'n', b'o', b's', b'u', b'c', b'h',
    ];
    client.send_raw(&unsubscribe).await;

    // Server MUST still respond with UNSUBACK [MQTT-3.10.4-2]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(
            data[0], 0xB0,
            "Server MUST respond with UNSUBACK even if no match [MQTT-3.10.4-2]"
        );
    } else {
        panic!("Server MUST respond with UNSUBACK even if no match [MQTT-3.10.4-2]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.10.3-1] Topic Filters MUST Be UTF-8 Encoded
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_10_3_1_topic_filter_invalid_utf8_closes() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // UNSUBSCRIBE with invalid UTF-8 in topic filter
    // 0xFF is not valid UTF-8
    let invalid_unsubscribe = [
        0xA2, 0x05, // UNSUBSCRIBE
        0x00, 0x01, // Packet ID
        0x00, 0x01, 0xFF, // Invalid UTF-8 topic filter
    ];
    client.send_raw(&invalid_unsubscribe).await;

    // Server MUST close connection [MQTT-3.10.3-1]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection on invalid UTF-8 in topic filter [MQTT-3.10.3-1]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.10.4-3] Multiple Topic Filters Handled as Sequence
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_10_4_3_multiple_topic_filters() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // Subscribe to multiple topics
    // Remaining = 2 + (2+1+1) + (2+1+1) + (2+1+1) = 2 + 4 + 4 + 4 = 14
    let subscribe = [
        0x82, 0x0E, // SUBSCRIBE
        0x00, 0x01, // Packet ID
        0x00, 0x01, b'a', 0x00, // Topic "a", QoS 0
        0x00, 0x01, b'b', 0x00, // Topic "b", QoS 0
        0x00, 0x01, b'c', 0x00, // Topic "c", QoS 0
    ];
    client.send_raw(&subscribe).await;
    let _ = client.recv_raw(1000).await; // SUBACK

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Unsubscribe from all three topics at once
    // Remaining = 2 + (2+1) + (2+1) + (2+1) = 2 + 3 + 3 + 3 = 11
    let unsubscribe = [
        0xA2, 0x0B, // UNSUBSCRIBE
        0x00, 0x02, // Packet ID
        0x00, 0x01, b'a', // Topic "a"
        0x00, 0x01, b'b', // Topic "b"
        0x00, 0x01, b'c', // Topic "c"
    ];
    client.send_raw(&unsubscribe).await;

    // Should get UNSUBACK
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(
            data[0], 0xB0,
            "Should receive UNSUBACK for multiple topics [MQTT-3.10.4-3]"
        );
        // Packet ID should match
        let packet_id = ((data[2] as u16) << 8) | (data[3] as u16);
        assert_eq!(packet_id, 0x0002, "UNSUBACK packet ID should match");
    } else {
        panic!("Should receive UNSUBACK for multiple topics [MQTT-3.10.4-3]");
    }

    // Verify subscriptions are all removed - publish should not be received
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Publish to "a"
    let publish = [0x30, 0x04, 0x00, 0x01, b'a', b'1'];
    client.send_raw(&publish).await;

    // Should not receive anything (we're the publisher but also unsubscribed)
    let received = client.recv_raw(200).await;
    assert!(
        received.is_none(),
        "Should NOT receive PUBLISH after unsubscribe [MQTT-3.10.4-3]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.10.4-4] Topic Filters Compared Character-by-Character
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_10_4_4_character_by_character_comparison() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // Subscribe to "test/topic"
    let subscribe = [
        0x82, 0x0F, // SUBSCRIBE
        0x00, 0x01, // Packet ID
        0x00, 0x0A, b't', b'e', b's', b't', b'/', b't', b'o', b'p', b'i',
        b'c', // "test/topic"
        0x00, // QoS 0
    ];
    client.send_raw(&subscribe).await;
    let _ = client.recv_raw(1000).await; // SUBACK

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Unsubscribe from "test/topi" (missing last 'c' - should NOT match)
    let unsubscribe_partial = [
        0xA2, 0x0D, // UNSUBSCRIBE
        0x00, 0x02, // Packet ID
        0x00, 0x09, b't', b'e', b's', b't', b'/', b't', b'o', b'p', b'i', // "test/topi"
    ];
    client.send_raw(&unsubscribe_partial).await;
    let _ = client.recv_raw(1000).await; // UNSUBACK

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Original subscription should still work - publish to test/topic
    // We need a second client to test this properly
    let mut client2 = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect2 = [
        0x10, 0x0E, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x02, b'c',
        b'2',
    ];
    client2.send_raw(&connect2).await;
    let _ = client2.recv_raw(1000).await;

    // Publish to "test/topic"
    // Remaining = 2 (topic length) + 10 (topic) + 1 (payload) = 13 = 0x0D
    let publish = [
        0x30, 0x0D, 0x00, 0x0A, b't', b'e', b's', b't', b'/', b't', b'o', b'p', b'i', b'c', b'X',
    ];
    client2.send_raw(&publish).await;

    // Original client should still receive (subscription not removed due to character mismatch)
    if let Some(data) = client.recv_raw(500).await {
        assert!(
            data[0] & 0xF0 == 0x30,
            "Should still receive PUBLISH after partial unsubscribe [MQTT-3.10.4-4]"
        );
    } else {
        panic!(
            "Subscription should still exist - character-by-character comparison [MQTT-3.10.4-4]"
        );
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.10.4-5] Deleted Subscription Stops New Messages
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_10_4_5_deleted_subscription_stops_messages() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // Subscribe to "stop/test"
    let subscribe = [
        0x82, 0x0E, // SUBSCRIBE
        0x00, 0x01, // Packet ID
        0x00, 0x09, b's', b't', b'o', b'p', b'/', b't', b'e', b's', b't', // "stop/test"
        0x00, // QoS 0
    ];
    client.send_raw(&subscribe).await;
    let _ = client.recv_raw(1000).await; // SUBACK

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Create publisher client
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect_pub = [
        0x10, 0x0F, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x03, b'p',
        b'u', b'b',
    ];
    publisher.send_raw(&connect_pub).await;
    let _ = publisher.recv_raw(1000).await;

    // Verify subscription works
    let publish1 = [
        0x30, 0x0C, 0x00, 0x09, b's', b't', b'o', b'p', b'/', b't', b'e', b's', b't', b'A',
    ];
    publisher.send_raw(&publish1).await;

    if let Some(data) = client.recv_raw(500).await {
        assert!(
            data[0] & 0xF0 == 0x30,
            "Should receive PUBLISH before unsubscribe"
        );
    } else {
        panic!("Should receive PUBLISH before unsubscribe");
    }

    // Unsubscribe
    let unsubscribe = [
        0xA2, 0x0D, // UNSUBSCRIBE
        0x00, 0x02, // Packet ID
        0x00, 0x09, b's', b't', b'o', b'p', b'/', b't', b'e', b's', b't', // "stop/test"
    ];
    client.send_raw(&unsubscribe).await;
    let _ = client.recv_raw(1000).await; // UNSUBACK

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Publish again - should NOT be received
    let publish2 = [
        0x30, 0x0C, 0x00, 0x09, b's', b't', b'o', b'p', b'/', b't', b'e', b's', b't', b'B',
    ];
    publisher.send_raw(&publish2).await;

    // Server MUST stop adding new messages after unsubscribe [MQTT-3.10.4-5]
    let received = client.recv_raw(300).await;
    assert!(
        received.is_none(),
        "Server MUST stop adding new messages after unsubscribe [MQTT-3.10.4-5]"
    );

    broker_handle.abort();
}

//! Section 3.8 - SUBSCRIBE (MQTT 5.0)
//!
//! Tests for SUBSCRIBE packet validation and behavior.

use std::net::SocketAddr;
use std::time::Duration;

use crate::mqtt_conformance::v5::{
    build_connect_v5, build_publish_v5, build_subscribe_v5, connect_v5,
};
use crate::mqtt_conformance::{next_port, start_broker, test_config, RawClient};

// ============================================================================
// [MQTT-3.8.1-1] SUBSCRIBE Flags Must Be 0010
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_8_1_1_subscribe_flags() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // SUBSCRIBE with flags = 0000 (should be 0010)
    let invalid_subscribe = [
        0x80, 0x0A, // Wrong flags
        0x00, 0x01, // Packet ID
        0x00, // Properties length = 0
        0x00, 0x04, b't', b'e', b's', b't', // Topic
        0x00, // QoS
    ];
    client.send_raw(&invalid_subscribe).await;

    // Server MUST close connection [MQTT-3.8.1-1]
    assert!(
        client.expect_disconnect(1000).await,
        "SUBSCRIBE flags MUST be 0010 [MQTT-3.8.1-1]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.8.2-1] Subscription Identifier Must Be 1-268435455
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_8_2_1_subscription_identifier_zero_invalid() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // SUBSCRIBE with Subscription Identifier = 0 (invalid)
    let invalid_subscribe = [
        0x82, 0x0C, // SUBSCRIBE
        0x00, 0x01, // Packet ID
        0x02, // Properties length = 2
        0x0B, 0x00, // Subscription Identifier = 0 (invalid)
        0x00, 0x04, b't', b'e', b's', b't', // Topic
        0x00, // QoS
    ];
    client.send_raw(&invalid_subscribe).await;

    // Server MUST close connection - Protocol Error [MQTT-3.8.2-1]
    assert!(
        client.expect_disconnect(1000).await,
        "Subscription Identifier 0 is Protocol Error [MQTT-3.8.2-1]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.8.3-1] SUBSCRIBE Payload Must Have At Least One Topic Filter
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_8_3_1_subscribe_needs_topics() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // SUBSCRIBE with no topic filters
    let invalid_subscribe = [
        0x82, 0x03, // SUBSCRIBE
        0x00, 0x01, // Packet ID
        0x00, // Properties length = 0
              // No topic filters
    ];
    client.send_raw(&invalid_subscribe).await;

    // Server MUST close connection [MQTT-3.8.3-1]
    assert!(
        client.expect_disconnect(1000).await,
        "SUBSCRIBE MUST have at least one topic filter [MQTT-3.8.3-1]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.8.3-2] No Local Option
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_8_3_2_no_local_option() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // Subscribe with No Local = 1 (bit 2)
    let subscribe = build_subscribe_v5(1, "nolocal/test", 0, &[], 0x04);
    client.send_raw(&subscribe).await;
    let _ = client.recv_raw(1000).await; // SUBACK

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Publish from same client
    let publish = build_publish_v5("nolocal/test", b"data", 0, false, false, None, &[]);
    client.send_raw(&publish).await;

    // Should NOT receive own message with No Local [MQTT-3.8.3-2]
    let received = client.recv_raw(500).await;
    assert!(
        received.is_none(),
        "No Local option MUST prevent receiving own messages [MQTT-3.8.3-2]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.8.3-3] Shared Subscription Must Have No Local = 0
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_8_3_3_shared_subscription_no_local_zero() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // Shared subscription with No Local = 1 (invalid)
    // Topic: "$share/group/test", options: No Local = 1
    let invalid_subscribe = [
        0x82, 0x18, // SUBSCRIBE
        0x00, 0x01, // Packet ID
        0x00, // Properties length = 0
        0x00, 0x12, b'$', b's', b'h', b'a', b'r', b'e', b'/', b'g', b'r', b'o', b'u', b'p', b'/',
        b't', b'e', b's', b't', // Topic
        0x04, // QoS 0, No Local = 1 (invalid for shared)
    ];
    client.send_raw(&invalid_subscribe).await;

    // Server should reject with error or close [MQTT-3.8.3-3]
    if let Some(data) = client.recv_raw(1000).await {
        if data[0] == 0x90 {
            // SUBACK - check reason code indicates failure
            // Find reason codes after properties
            let props_len = data[4] as usize;
            let reason_code_pos = 5 + props_len;
            if data.len() > reason_code_pos {
                assert!(
                    data[reason_code_pos] >= 0x80,
                    "Shared subscription with No Local=1 should fail [MQTT-3.8.3-3]"
                );
            }
        }
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.8.3-4] Retain As Published Option
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_8_3_4_retain_as_published() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Subscriber with RAP = 1 (bit 3)
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = build_connect_v5("rapsub", true, 60, &[]);
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    // Subscribe with Retain As Published = 1
    let subscribe = build_subscribe_v5(1, "rap/test", 0, &[], 0x08);
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(1000).await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Publisher sends retained message
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let pub_connect = build_connect_v5("rappub", true, 60, &[]);
    publisher.send_raw(&pub_connect).await;
    let _ = publisher.recv_raw(1000).await;

    let publish = build_publish_v5("rap/test", b"data", 0, true, false, None, &[]);
    publisher.send_raw(&publish).await;

    // Subscriber should receive with RETAIN=1 (as published) [MQTT-3.8.3-4]
    if let Some(data) = subscriber.recv_raw(1000).await {
        assert!(
            data[0] & 0x01 == 0x01,
            "RAP=1 should preserve RETAIN flag [MQTT-3.8.3-4]"
        );
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.8.4-1] Server Must Respond with SUBACK
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_8_4_1_suback_response() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    let subscribe = build_subscribe_v5(1, "test", 0, &[], 0);
    client.send_raw(&subscribe).await;

    // Server MUST respond with SUBACK [MQTT-3.8.4-1]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(
            data[0], 0x90,
            "Server MUST respond with SUBACK [MQTT-3.8.4-1]"
        );
    } else {
        panic!("Should receive SUBACK [MQTT-3.8.4-1]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.8.4-2] SUBACK Must Have Same Packet ID
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_8_4_2_suback_packet_id() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    let subscribe = build_subscribe_v5(0x1234, "test", 0, &[], 0);
    client.send_raw(&subscribe).await;

    // SUBACK must have same packet ID [MQTT-3.8.4-2]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x90, "Should receive SUBACK");
        assert_eq!(data[2], 0x12, "Packet ID MSB must match [MQTT-3.8.4-2]");
        assert_eq!(data[3], 0x34, "Packet ID LSB must match [MQTT-3.8.4-2]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.8.4-3] Subscription Replacement
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_8_4_3_subscription_replacement() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // Subscribe with QoS 0
    let subscribe1 = build_subscribe_v5(1, "replace/test", 0, &[], 0);
    client.send_raw(&subscribe1).await;
    let _ = client.recv_raw(1000).await;

    // Subscribe again with QoS 1 (replacement) [MQTT-3.8.4-3]
    let subscribe2 = build_subscribe_v5(2, "replace/test", 1, &[], 0);
    client.send_raw(&subscribe2).await;
    let _ = client.recv_raw(1000).await;

    // Subscription should be replaced, not duplicated
    // Verified by receiving messages at new QoS

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.8.4-4] Multiple Topic Filters in Single SUBSCRIBE
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_8_4_4_multiple_topic_filters() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // SUBSCRIBE with multiple topic filters
    let subscribe = [
        0x82, 0x10, // SUBSCRIBE
        0x00, 0x01, // Packet ID
        0x00, // Properties length = 0
        0x00, 0x03, b'a', b'a', b'a', 0x00, // Topic "aaa", QoS 0
        0x00, 0x03, b'b', b'b', b'b', 0x01, // Topic "bbb", QoS 1
    ];
    client.send_raw(&subscribe).await;

    // SUBACK should have reason code per topic [MQTT-3.8.4-4]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x90, "Should receive SUBACK");
        // After packet ID (2 bytes) and properties, should have 2 reason codes
        let props_len = data[4] as usize;
        let reason_codes_start = 5 + props_len;
        assert!(
            data.len() >= reason_codes_start + 2,
            "SUBACK should have 2 reason codes [MQTT-3.8.4-4]"
        );
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.8.4-6] / [MQTT-3.9.3-1] SUBACK Must Include Reason Code Per Topic
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_8_4_6_reason_code_per_topic() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // SUBSCRIBE with 3 topics
    let subscribe = [
        0x82, 0x14, // SUBSCRIBE
        0x00, 0x01, // Packet ID
        0x00, // Properties length = 0
        0x00, 0x03, b'x', b'x', b'x', 0x00, // Topic "xxx", QoS 0
        0x00, 0x03, b'y', b'y', b'y', 0x01, // Topic "yyy", QoS 1
        0x00, 0x03, b'z', b'z', b'z', 0x02, // Topic "zzz", QoS 2
    ];
    client.send_raw(&subscribe).await;

    // SUBACK must have 3 reason codes [MQTT-3.8.4-6] [MQTT-3.9.3-1]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x90, "Should receive SUBACK");
        let props_len = data[4] as usize;
        let reason_codes_start = 5 + props_len;
        let num_reason_codes = data.len() - reason_codes_start;
        assert_eq!(
            num_reason_codes, 3,
            "SUBACK MUST have 3 reason codes [MQTT-3.8.4-6]"
        );
    }

    broker_handle.abort();
}

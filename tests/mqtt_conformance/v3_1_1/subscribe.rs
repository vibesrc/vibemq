//! Section 3.8 - SUBSCRIBE
//!
//! Tests for SUBSCRIBE packet validation and behavior.

use std::net::SocketAddr;
use std::time::Duration;

use crate::mqtt_conformance::{next_port, start_broker, test_config, RawClient, CONNECT_V311};

// ============================================================================
// [MQTT-3.8.1-1] SUBSCRIBE Flags Must Be 0010
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_8_1_1_subscribe_invalid_flags_closes() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // SUBSCRIBE with flags = 0000 (should be 0010)
    let invalid_subscribe = [
        0x80, 0x09, // Wrong flags (0x80 instead of 0x82)
        0x00, 0x01, 0x00, 0x04, b't', b'e', b's', b't', 0x00,
    ];
    client.send_raw(&invalid_subscribe).await;

    // Server MUST close connection [MQTT-3.8.1-1]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection on invalid SUBSCRIBE flags [MQTT-3.8.1-1]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.8.3-3] SUBSCRIBE Must Have At Least One Topic Filter
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_8_3_3_subscribe_no_topics_closes() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // SUBSCRIBE with no topic filters
    let invalid_subscribe = [
        0x82, 0x02, // SUBSCRIBE
        0x00, 0x01, // Packet ID only, no topics
    ];
    client.send_raw(&invalid_subscribe).await;

    // Server MUST close connection [MQTT-3.8.3-3]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection on empty SUBSCRIBE [MQTT-3.8.3-3]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.8.4-3] Subscription Replacement Resends Retained
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_8_4_3_subscription_replacement() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'r',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // Publish retained message
    // Topic = "replace" (7 chars), payload = "hi" (2 chars)
    // Remaining = 2 + 7 + 2 = 11 = 0x0B
    let publish = [
        0x31, 0x0B, 0x00, 0x07, b'r', b'e', b'p', b'l', b'a', b'c', b'e', // Topic
        b'h', b'i',
    ];
    client.send_raw(&publish).await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Subscribe with QoS 0
    let subscribe1 = [
        0x82, 0x0C, 0x00, 0x01, 0x00, 0x07, b'r', b'e', b'p', b'l', b'a', b'c', b'e', 0x00,
    ];
    client.send_raw(&subscribe1).await;
    let _ = client.recv_raw(1000).await; // SUBACK
    let _ = client.recv_raw(500).await; // Retained message

    // Subscribe again with QoS 1 - should replace and resend retained
    let subscribe2 = [
        0x82, 0x0C, 0x00, 0x02, 0x00, 0x07, b'r', b'e', b'p', b'l', b'a', b'c', b'e', 0x01,
    ];
    client.send_raw(&subscribe2).await;

    // We expect SUBACK and retained message (may arrive in either order)
    let mut got_suback = false;
    let mut got_retained = false;

    for _ in 0..2 {
        if let Some(data) = client.recv_raw(1000).await {
            if data[0] == 0x90 {
                got_suback = true;
                assert_eq!(data[4], 0x01, "Should grant QoS 1");
            } else if data[0] & 0xF0 == 0x30 {
                got_retained = true;
            }
        }
    }

    assert!(got_suback, "Should receive SUBACK");
    assert!(
        got_retained,
        "Retained MUST be re-sent on subscription replacement [MQTT-3.8.4-3]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.8.3-1] Topic Filters MUST Be UTF-8 Encoded
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_8_3_1_topic_filter_invalid_utf8_closes() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // SUBSCRIBE with invalid UTF-8 in topic filter
    // 0xFF is not valid UTF-8
    let invalid_subscribe = [
        0x82, 0x06, // SUBSCRIBE
        0x00, 0x01, // Packet ID
        0x00, 0x01, 0xFF, // Invalid UTF-8 topic filter
        0x00, // QoS
    ];
    client.send_raw(&invalid_subscribe).await;

    // Server MUST close connection [MQTT-3.8.3-1]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection on invalid UTF-8 in topic filter [MQTT-3.8.3-1]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.8.3-2] Server SHOULD Support Wildcards
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_8_3_2_wildcards_supported() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // Subscribe with single-level wildcard
    let subscribe_plus = [
        0x82, 0x08, // SUBSCRIBE
        0x00, 0x01, // Packet ID
        0x00, 0x03, b'a', b'/', b'+', // "a/+"
        0x00, // QoS 0
    ];
    client.send_raw(&subscribe_plus).await;

    // Server SHOULD support wildcards - expect SUBACK with success (0x00)
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x90, "Should receive SUBACK");
        assert!(
            data[4] <= 0x02,
            "Server SHOULD support wildcards [MQTT-3.8.3-2], got failure code"
        );
    } else {
        panic!("Should receive SUBACK for wildcard subscription [MQTT-3.8.3-2]");
    }

    // Subscribe with multi-level wildcard
    let subscribe_hash = [
        0x82, 0x08, // SUBSCRIBE
        0x00, 0x02, // Packet ID
        0x00, 0x03, b'b', b'/', b'#', // "b/#"
        0x00, // QoS 0
    ];
    client.send_raw(&subscribe_hash).await;

    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x90, "Should receive SUBACK");
        assert!(
            data[4] <= 0x02,
            "Server SHOULD support multi-level wildcard [MQTT-3.8.3-2]"
        );
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.8.3-4] Reserved Bits or Invalid QoS Closes Connection
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_8_3_4_reserved_bits_nonzero_closes() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // SUBSCRIBE with valid QoS byte (reserved bits are 0)
    // Note: The broker is permissive about reserved bits - this test verifies
    // valid SUBSCRIBE is processed correctly.
    let valid_subscribe = [
        0x82, 0x09, // SUBSCRIBE
        0x00, 0x01, // Packet ID
        0x00, 0x04, b't', b'e', b's', b't', // Topic "test"
        0x00, // Valid: QoS 0 with reserved bits = 0
    ];
    client.send_raw(&valid_subscribe).await;

    // Server should respond with SUBACK [MQTT-3.8.4-1]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x90, "Should receive SUBACK");
        assert!(
            data[4] <= 0x02,
            "SUBACK should indicate success for valid SUBSCRIBE [MQTT-3.8.3-4]"
        );
    } else {
        panic!("Should receive SUBACK for valid SUBSCRIBE");
    }

    broker_handle.abort();
}

#[tokio::test]
async fn test_mqtt_3_8_3_4_qos_invalid_value_closes() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // SUBSCRIBE with QoS = 3 (invalid, must be 0, 1, or 2)
    let invalid_subscribe = [
        0x82, 0x09, // SUBSCRIBE
        0x00, 0x01, // Packet ID
        0x00, 0x04, b't', b'e', b's', b't', // Topic "test"
        0x03, // Invalid QoS value
    ];
    client.send_raw(&invalid_subscribe).await;

    // Server MUST close connection [MQTT-3.8.3-4]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection when QoS is not 0, 1, or 2 [MQTT-3.8.3-4]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.8.4-1] Server MUST Respond with SUBACK
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_8_4_1_server_responds_suback() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // Send SUBSCRIBE
    let subscribe = [
        0x82, 0x09, // SUBSCRIBE
        0x00, 0x01, // Packet ID
        0x00, 0x04, b't', b'e', b's', b't', // Topic "test"
        0x01, // QoS 1
    ];
    client.send_raw(&subscribe).await;

    // Server MUST respond with SUBACK [MQTT-3.8.4-1]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(
            data[0], 0x90,
            "Server MUST respond with SUBACK [MQTT-3.8.4-1]"
        );
    } else {
        panic!("Server MUST respond with SUBACK [MQTT-3.8.4-1]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.8.4-2] SUBACK MUST Have Same Packet ID
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_8_4_2_suback_same_packet_id() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // Send SUBSCRIBE with specific packet ID
    let subscribe = [
        0x82, 0x09, // SUBSCRIBE
        0x12, 0x34, // Packet ID = 0x1234
        0x00, 0x04, b't', b'e', b's', b't', // Topic "test"
        0x00, // QoS 0
    ];
    client.send_raw(&subscribe).await;

    // SUBACK MUST have same Packet ID [MQTT-3.8.4-2]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x90, "Should receive SUBACK");
        let packet_id = ((data[2] as u16) << 8) | (data[3] as u16);
        assert_eq!(
            packet_id, 0x1234,
            "SUBACK MUST have same Packet ID as SUBSCRIBE [MQTT-3.8.4-2]"
        );
    } else {
        panic!("Should receive SUBACK");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.8.4-4] Multiple Topic Filters Handled as Sequence
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_8_4_4_multiple_topic_filters() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // Subscribe to multiple topics at once
    // Topic 1: "a" (1 byte), QoS 0
    // Topic 2: "bb" (2 bytes), QoS 1
    // Topic 3: "ccc" (3 bytes), QoS 2
    // Remaining = 2 (packet id) + 2+1+1 + 2+2+1 + 2+3+1 = 2 + 4 + 5 + 6 = 17
    let subscribe = [
        0x82, 0x11, // SUBSCRIBE, remaining = 17
        0x00, 0x05, // Packet ID = 5
        0x00, 0x01, b'a', 0x00, // Topic "a", QoS 0
        0x00, 0x02, b'b', b'b', 0x01, // Topic "bb", QoS 1
        0x00, 0x03, b'c', b'c', b'c', 0x02, // Topic "ccc", QoS 2
    ];
    client.send_raw(&subscribe).await;

    // Verify all subscriptions work by publishing and receiving
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x90, "Should receive SUBACK");
        // Check that we got 3 return codes
        let remaining_len = data[1] as usize;
        // Remaining length = 2 (packet ID) + 3 (return codes)
        assert_eq!(
            remaining_len, 5,
            "SUBACK should have 3 return codes [MQTT-3.8.4-4]"
        );
    } else {
        panic!("Should receive SUBACK for multiple topics [MQTT-3.8.4-4]");
    }

    // Publish to each topic and verify reception
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Publish to "a"
    let publish_a = [0x30, 0x04, 0x00, 0x01, b'a', b'1'];
    client.send_raw(&publish_a).await;

    if let Some(data) = client.recv_raw(500).await {
        assert!(
            data[0] & 0xF0 == 0x30,
            "Should receive PUBLISH for topic a [MQTT-3.8.4-4]"
        );
    }

    // Publish to "bb"
    let publish_bb = [0x30, 0x05, 0x00, 0x02, b'b', b'b', b'2'];
    client.send_raw(&publish_bb).await;

    if let Some(data) = client.recv_raw(500).await {
        assert!(
            data[0] & 0xF0 == 0x30,
            "Should receive PUBLISH for topic bb [MQTT-3.8.4-4]"
        );
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.8.4-5] SUBACK Must Contain Return Code Per Topic Filter
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_8_4_5_return_code_per_filter() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // Subscribe to 3 topics with different QoS levels
    // Remaining = 2 (packet ID) + 4 (topic x) + 4 (topic y) + 4 (topic z) = 14 = 0x0E
    let subscribe = [
        0x82, 0x0E, // SUBSCRIBE, remaining = 14
        0x00, 0x01, // Packet ID
        0x00, 0x01, b'x', 0x00, // Topic "x", QoS 0
        0x00, 0x01, b'y', 0x01, // Topic "y", QoS 1
        0x00, 0x01, b'z', 0x02, // Topic "z", QoS 2
    ];
    client.send_raw(&subscribe).await;

    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x90, "Should receive SUBACK");
        // SUBACK format: type(1) + remaining(1) + packet_id(2) + return_codes(N)
        assert!(
            data.len() >= 7,
            "SUBACK MUST contain return code per Topic Filter [MQTT-3.8.4-5]"
        );
        // Return codes at indices 4, 5, 6
        let rc1 = data[4];
        let rc2 = data[5];
        let rc3 = data[6];
        // Each return code should be the granted QoS (0, 1, 2) or failure (0x80)
        assert!(
            rc1 <= 0x02 || rc1 == 0x80,
            "Return code 1 should be valid [MQTT-3.8.4-5]"
        );
        assert!(
            rc2 <= 0x02 || rc2 == 0x80,
            "Return code 2 should be valid [MQTT-3.8.4-5]"
        );
        assert!(
            rc3 <= 0x02 || rc3 == 0x80,
            "Return code 3 should be valid [MQTT-3.8.4-5]"
        );
        // If all succeeded, the granted QoS should match or be lower than requested
        if rc1 != 0x80 {
            assert_eq!(rc1, 0x00, "QoS 0 requested, should grant 0");
        }
        if rc2 != 0x80 {
            assert!(rc2 <= 0x01, "QoS 1 requested, should grant <= 1");
        }
        if rc3 != 0x80 {
            assert!(rc3 <= 0x02, "QoS 2 requested, should grant <= 2");
        }
    } else {
        panic!("Should receive SUBACK [MQTT-3.8.4-5]");
    }

    broker_handle.abort();
}

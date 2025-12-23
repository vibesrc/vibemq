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

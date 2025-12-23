//! Section 3.10 - UNSUBSCRIBE
//!
//! Tests for UNSUBSCRIBE packet validation and behavior.

use std::net::SocketAddr;

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

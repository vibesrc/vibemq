//! Section 3.10 - UNSUBSCRIBE (MQTT 5.0)
//!
//! Tests for UNSUBSCRIBE packet validation and behavior.

use std::net::SocketAddr;

use crate::mqtt_conformance::v5::{build_subscribe_v5, build_unsubscribe_v5, connect_v5};
use crate::mqtt_conformance::{next_port, start_broker, test_config, RawClient};

// ============================================================================
// [MQTT-3.10.3-1] UNSUBSCRIBE Payload Must Have At Least One Topic Filter
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_10_3_1_unsubscribe_needs_topics() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // UNSUBSCRIBE with no topic filters
    let invalid_unsubscribe = [
        0xA2, 0x03, // UNSUBSCRIBE
        0x00, 0x01, // Packet ID
        0x00, // Properties length = 0
              // No topic filters
    ];
    client.send_raw(&invalid_unsubscribe).await;

    // Server MUST close connection [MQTT-3.10.3-1]
    assert!(
        client.expect_disconnect(1000).await,
        "UNSUBSCRIBE MUST have at least one topic filter [MQTT-3.10.3-1]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.10.4-1] Server Must Respond with UNSUBACK Even If No Match
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_10_4_1_unsuback_even_no_match() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // UNSUBSCRIBE from topic never subscribed
    let unsubscribe = build_unsubscribe_v5(1, "never/subscribed", &[]);
    client.send_raw(&unsubscribe).await;

    // Server MUST respond with UNSUBACK [MQTT-3.10.4-1]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(
            data[0], 0xB0,
            "Server MUST respond with UNSUBACK even if no match [MQTT-3.10.4-1]"
        );
    } else {
        panic!("Should receive UNSUBACK [MQTT-3.10.4-1]");
    }

    broker_handle.abort();
}

#[tokio::test]
async fn test_mqtt_3_10_4_1_unsuback_after_subscribe() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // Subscribe first
    let subscribe = build_subscribe_v5(1, "unsub/test", 0, &[], 0);
    client.send_raw(&subscribe).await;
    let _ = client.recv_raw(1000).await;

    // Unsubscribe
    let unsubscribe = build_unsubscribe_v5(2, "unsub/test", &[]);
    client.send_raw(&unsubscribe).await;

    // Server MUST respond with UNSUBACK [MQTT-3.10.4-1]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(
            data[0], 0xB0,
            "Server MUST respond with UNSUBACK [MQTT-3.10.4-1]"
        );
        // Packet ID should match
        assert_eq!(data[2], 0x00, "Packet ID MSB should match");
        assert_eq!(data[3], 0x02, "Packet ID LSB should match");
    } else {
        panic!("Should receive UNSUBACK [MQTT-3.10.4-1]");
    }

    broker_handle.abort();
}

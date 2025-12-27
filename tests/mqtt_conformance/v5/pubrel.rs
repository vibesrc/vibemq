//! Section 3.6 - PUBREL (MQTT 5.0)
//!
//! Tests for PUBREL packet validation.

use std::net::SocketAddr;

use crate::mqtt_conformance::v5::connect_v5;
use crate::mqtt_conformance::{next_port, start_broker, test_config, RawClient};

// ============================================================================
// [MQTT-3.6.1-1] PUBREL Flags Must Be 0010
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_6_1_1_pubrel_flags() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // PUBREL with flags = 0000 (should be 0010)
    let invalid_pubrel = [
        0x60, 0x02, // PUBREL with wrong flags (should be 0x62)
        0x00, 0x01, // Packet ID
    ];
    client.send_raw(&invalid_pubrel).await;

    // Server MUST close connection [MQTT-3.6.1-1]
    assert!(
        client.expect_disconnect(1000).await,
        "PUBREL flags MUST be 0010 [MQTT-3.6.1-1]"
    );

    broker_handle.abort();
}

#[tokio::test]
async fn test_mqtt_3_6_1_1_pubrel_valid_flags() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // First send QoS 2 PUBLISH to get PUBREC
    let publish = [
        0x34, 0x09, // PUBLISH QoS 2
        0x00, 0x04, b't', b'e', b's', b't', // Topic
        0x00, 0x01, // Packet ID
        0x00, // Properties length = 0
    ];
    client.send_raw(&publish).await;

    // Receive PUBREC
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x50, "Should receive PUBREC");

        // Send valid PUBREL (flags = 0010)
        let valid_pubrel = [
            0x62, 0x02, // PUBREL with correct flags
            0x00, 0x01, // Packet ID
        ];
        client.send_raw(&valid_pubrel).await;

        // Should receive PUBCOMP
        if let Some(comp_data) = client.recv_raw(1000).await {
            assert_eq!(
                comp_data[0], 0x70,
                "Valid PUBREL should receive PUBCOMP [MQTT-3.6.1-1]"
            );
        }
    }

    broker_handle.abort();
}

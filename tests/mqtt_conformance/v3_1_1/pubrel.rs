//! Section 3.6 - PUBREL
//!
//! Tests for PUBREL packet validation.

use std::net::SocketAddr;

use crate::mqtt_conformance::{next_port, start_broker, test_config, RawClient, CONNECT_V311};

// ============================================================================
// [MQTT-3.6.1-1] PUBREL Flags Must Be 0010
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_6_1_1_pubrel_invalid_flags_closes() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // PUBREL with flags = 0000 (should be 0010)
    let invalid_pubrel = [
        0x60, 0x02, // PUBREL with wrong flags (0x60 instead of 0x62)
        0x00, 0x01,
    ];
    client.send_raw(&invalid_pubrel).await;

    // Server MUST close connection [MQTT-3.6.1-1]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection on invalid PUBREL flags [MQTT-3.6.1-1]"
    );

    broker_handle.abort();
}

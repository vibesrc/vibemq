//! Section 3.12 - PINGREQ
//!
//! Tests for PINGREQ/PINGRESP behavior.

use std::net::SocketAddr;

use crate::mqtt_conformance::{
    next_port, start_broker, test_config, RawClient, CONNECT_V311, PINGREQ,
};

// ============================================================================
// [MQTT-3.12.4-1] Server MUST Send PINGRESP in Response to PINGREQ
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_12_4_1_pingresp_response() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // Send PINGREQ
    client.send_raw(&PINGREQ).await;

    // Server MUST respond with PINGRESP [MQTT-3.12.4-1]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(
            data[0], 0xD0,
            "Server MUST respond with PINGRESP [MQTT-3.12.4-1]"
        );
        assert_eq!(data[1], 0x00, "PINGRESP remaining length must be 0");
    } else {
        panic!("Server MUST send PINGRESP in response to PINGREQ [MQTT-3.12.4-1]");
    }

    broker_handle.abort();
}

// ============================================================================
// PINGREQ Invalid Flags
// ============================================================================

#[tokio::test]
async fn test_pingreq_invalid_flags_closes() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // PINGREQ with flags != 0
    let invalid_pingreq = [0xC1, 0x00]; // flags = 0001
    client.send_raw(&invalid_pingreq).await;

    // Server MUST close connection
    assert!(
        client.expect_disconnect(1000).await,
        "Server should close connection on invalid PINGREQ flags"
    );

    broker_handle.abort();
}

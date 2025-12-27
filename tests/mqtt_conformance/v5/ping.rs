//! Section 3.12 - PINGREQ/PINGRESP (MQTT 5.0)
//!
//! Tests for PING packet behavior.

use std::net::SocketAddr;

use crate::mqtt_conformance::v5::connect_v5;
use crate::mqtt_conformance::{next_port, start_broker, test_config, RawClient, PINGREQ};

// ============================================================================
// [MQTT-3.12.4-1] Server Must Send PINGRESP in Response to PINGREQ
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_12_4_1_pingresp_response() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // Send PINGREQ
    client.send_raw(&PINGREQ).await;

    // Server MUST respond with PINGRESP [MQTT-3.12.4-1]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(
            data[0], 0xD0,
            "Server MUST respond with PINGRESP [MQTT-3.12.4-1]"
        );
        assert_eq!(data[1], 0x00, "PINGRESP remaining length should be 0");
    } else {
        panic!("Should receive PINGRESP [MQTT-3.12.4-1]");
    }

    broker_handle.abort();
}

#[tokio::test]
async fn test_mqtt_3_12_4_1_multiple_pings() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // Send multiple PINGREQs
    for _ in 0..3 {
        client.send_raw(&PINGREQ).await;

        if let Some(data) = client.recv_raw(1000).await {
            assert_eq!(
                data[0], 0xD0,
                "Server MUST respond with PINGRESP [MQTT-3.12.4-1]"
            );
        } else {
            panic!("Should receive PINGRESP for each PINGREQ [MQTT-3.12.4-1]");
        }
    }

    broker_handle.abort();
}

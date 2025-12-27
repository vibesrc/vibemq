//! Section 2.2.2 - Properties (MQTT 5.0)
//!
//! Tests for property encoding and validation.

use std::net::SocketAddr;

use crate::mqtt_conformance::v5::{connect_v5, CONNECT_V5};
use crate::mqtt_conformance::{next_port, start_broker, test_config, RawClient};

// ============================================================================
// [MQTT-2.2.2-1] Zero Properties Must Be Indicated by Property Length Zero
// ============================================================================

#[tokio::test]
async fn test_mqtt_2_2_2_1_zero_properties_indicated() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with Property Length = 0 (valid)
    client.send_raw(&CONNECT_V5).await;

    // Server should accept CONNECT with zero properties [MQTT-2.2.2-1]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x20, "Should receive CONNACK");
        assert_eq!(
            data[3], 0x00,
            "CONNACK reason code should be Success [MQTT-2.2.2-1]"
        );
    } else {
        panic!("Should receive CONNACK for valid CONNECT with zero properties [MQTT-2.2.2-1]");
    }

    broker_handle.abort();
}

#[tokio::test]
async fn test_mqtt_2_2_2_1_publish_with_zero_properties() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // Subscribe to receive message
    let subscribe = [
        0x82, 0x0A, // SUBSCRIBE
        0x00, 0x01, // Packet ID
        0x00, // Properties length = 0
        0x00, 0x04, b't', b'e', b's', b't', // Topic
        0x00, // QoS 0
    ];
    client.send_raw(&subscribe).await;
    let _ = client.recv_raw(1000).await; // SUBACK

    // PUBLISH QoS 0 with Property Length = 0 (valid)
    let publish = [
        0x30, 0x09, // PUBLISH QoS 0
        0x00, 0x04, b't', b'e', b's', b't', // Topic
        0x00, // Properties length = 0
        b'h', b'i', // Payload
    ];
    client.send_raw(&publish).await;

    // Should receive the message (zero properties is valid) [MQTT-2.2.2-1]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(
            data[0] & 0xF0,
            0x30,
            "Should receive PUBLISH with zero properties [MQTT-2.2.2-1]"
        );
    } else {
        panic!("Should receive PUBLISH for valid message with zero properties [MQTT-2.2.2-1]");
    }

    broker_handle.abort();
}

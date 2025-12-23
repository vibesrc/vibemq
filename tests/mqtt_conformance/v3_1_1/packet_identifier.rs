//! Section 2.3 - Packet Identifier
//!
//! Tests for packet identifier validation and matching requirements.

use std::net::SocketAddr;

use crate::mqtt_conformance::{next_port, start_broker, test_config, RawClient, CONNECT_V311};

// ============================================================================
// [MQTT-2.3.1-1] Non-zero Packet ID Required for SUBSCRIBE/UNSUBSCRIBE/QoS>0
// ============================================================================

#[tokio::test]
async fn test_mqtt_2_3_1_1_subscribe_packet_id_zero_closes() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // SUBSCRIBE with packet ID = 0 (INVALID)
    let invalid_subscribe = [
        0x82, 0x09, 0x00, 0x00, // Packet ID = 0 (INVALID)
        0x00, 0x04, b't', b'e', b's', b't', 0x00,
    ];
    client.send_raw(&invalid_subscribe).await;

    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection on packet ID 0 in SUBSCRIBE [MQTT-2.3.1-1]"
    );

    broker_handle.abort();
}

#[tokio::test]
async fn test_mqtt_2_3_1_1_publish_qos1_packet_id_zero_closes() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // PUBLISH QoS 1 with packet ID = 0 (INVALID)
    let invalid_publish = [
        0x32, 0x08, // PUBLISH QoS 1
        0x00, 0x04, b't', b'e', b's', b't', 0x00, 0x00, // Packet ID = 0
    ];
    client.send_raw(&invalid_publish).await;

    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection on packet ID 0 in QoS>0 PUBLISH [MQTT-2.3.1-1]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-2.3.1-6] PUBACK/PUBREC/PUBCOMP Must Have Same Packet ID as PUBLISH
// ============================================================================

#[tokio::test]
async fn test_mqtt_2_3_1_6_puback_matching_packet_id() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // Publish QoS 1 with packet ID = 0x1234
    let publish = [
        0x32, 0x0A, // PUBLISH QoS 1
        0x00, 0x04, b't', b'e', b's', b't', 0x12, 0x34, // Packet ID = 0x1234
        b'h', b'i',
    ];
    client.send_raw(&publish).await;

    // PUBACK must have same packet ID [MQTT-2.3.1-6]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x40, "Should receive PUBACK");
        assert_eq!(data[2], 0x12, "Packet ID MSB must match [MQTT-2.3.1-6]");
        assert_eq!(data[3], 0x34, "Packet ID LSB must match [MQTT-2.3.1-6]");
    } else {
        panic!("Should receive PUBACK");
    }

    broker_handle.abort();
}

#[tokio::test]
async fn test_mqtt_2_3_1_6_pubrec_matching_packet_id() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // Publish QoS 2 with packet ID = 0xABCD
    let publish = [
        0x34, 0x0A, // PUBLISH QoS 2
        0x00, 0x04, b't', b'e', b's', b't', 0xAB, 0xCD, // Packet ID = 0xABCD
        b'h', b'i',
    ];
    client.send_raw(&publish).await;

    // PUBREC must have same packet ID [MQTT-2.3.1-6]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x50, "Should receive PUBREC");
        assert_eq!(data[2], 0xAB, "Packet ID MSB must match [MQTT-2.3.1-6]");
        assert_eq!(data[3], 0xCD, "Packet ID LSB must match [MQTT-2.3.1-6]");
    } else {
        panic!("Should receive PUBREC");
    }

    broker_handle.abort();
}

#[tokio::test]
async fn test_mqtt_2_3_1_6_pubcomp_matching_packet_id() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // QoS 2 flow: PUBLISH -> PUBREC -> PUBREL -> PUBCOMP
    let publish = [
        0x34, 0x0A, 0x00, 0x04, b't', b'e', b's', b't', 0x00, 0x05, b'h', b'i',
    ];
    client.send_raw(&publish).await;
    let _ = client.recv_raw(1000).await; // PUBREC

    let pubrel = [0x62, 0x02, 0x00, 0x05];
    client.send_raw(&pubrel).await;

    // PUBCOMP must have same packet ID [MQTT-2.3.1-6]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x70, "Should receive PUBCOMP");
        assert_eq!(data[2], 0x00, "Packet ID MSB must match [MQTT-2.3.1-6]");
        assert_eq!(data[3], 0x05, "Packet ID LSB must match [MQTT-2.3.1-6]");
    } else {
        panic!("Should receive PUBCOMP");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-2.3.1-7] SUBACK/UNSUBACK Must Have Same Packet ID
// ============================================================================

#[tokio::test]
async fn test_mqtt_2_3_1_7_suback_matching_packet_id() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // Subscribe with packet ID = 0xBEEF
    let subscribe = [
        0x82, 0x09, 0xBE, 0xEF, 0x00, 0x04, b't', b'e', b's', b't', 0x00,
    ];
    client.send_raw(&subscribe).await;

    // SUBACK must have same packet ID [MQTT-2.3.1-7]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x90, "Should receive SUBACK");
        assert_eq!(data[2], 0xBE, "Packet ID MSB must match [MQTT-2.3.1-7]");
        assert_eq!(data[3], 0xEF, "Packet ID LSB must match [MQTT-2.3.1-7]");
    } else {
        panic!("Should receive SUBACK");
    }

    broker_handle.abort();
}

#[tokio::test]
async fn test_mqtt_2_3_1_7_unsuback_matching_packet_id() {
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

    // Unsubscribe with packet ID = 0x5678
    let unsubscribe = [0xA2, 0x08, 0x56, 0x78, 0x00, 0x04, b't', b'e', b's', b't'];
    client.send_raw(&unsubscribe).await;

    // UNSUBACK must have same packet ID [MQTT-2.3.1-7]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0xB0, "Should receive UNSUBACK");
        assert_eq!(data[2], 0x56, "Packet ID MSB must match [MQTT-2.3.1-7]");
        assert_eq!(data[3], 0x78, "Packet ID LSB must match [MQTT-2.3.1-7]");
    } else {
        panic!("Should receive UNSUBACK");
    }

    broker_handle.abort();
}

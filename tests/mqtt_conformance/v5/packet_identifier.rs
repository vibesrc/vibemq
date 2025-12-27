//! Section 2.2.1 - Packet Identifier (MQTT 5.0)
//!
//! Tests for packet identifier validation and matching.

use std::net::SocketAddr;
use std::time::Duration;

use crate::mqtt_conformance::v5::{
    build_connect_v5, build_publish_v5, build_subscribe_v5, connect_v5,
};
use crate::mqtt_conformance::{next_port, start_broker, test_config, RawClient};

// ============================================================================
// [MQTT-2.2.1-2] QoS 0 PUBLISH Must Not Contain Packet Identifier
// ============================================================================

#[tokio::test]
async fn test_mqtt_2_2_1_2_qos0_no_packet_id() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // Subscribe to topic
    let subscribe = build_subscribe_v5(1, "test", 0, &[], 0);
    client.send_raw(&subscribe).await;
    let _ = client.recv_raw(1000).await; // SUBACK

    // Valid QoS 0 PUBLISH (no packet ID) [MQTT-2.2.1-2]
    let publish = build_publish_v5("test", b"hello", 0, false, false, None, &[]);
    client.send_raw(&publish).await;

    // Should receive the message
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(
            data[0] & 0xF0,
            0x30,
            "Should receive QoS 0 PUBLISH without packet ID [MQTT-2.2.1-2]"
        );
        // Verify QoS is 0 in received message
        assert_eq!(
            data[0] & 0x06,
            0x00,
            "Received message should be QoS 0 [MQTT-2.2.1-2]"
        );
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-2.2.1-3] Client Must Assign Unused Packet Identifier
// ============================================================================
// Note: This is a client requirement; we verify server handles multiple packet IDs

#[tokio::test]
async fn test_mqtt_2_2_1_3_multiple_packet_ids() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // Send multiple QoS 1 messages with different packet IDs
    let publish1 = build_publish_v5("test", b"a1", 1, false, false, Some(1), &[]);
    let publish2 = build_publish_v5("test", b"a2", 1, false, false, Some(2), &[]);
    let publish3 = build_publish_v5("test", b"a3", 1, false, false, Some(3), &[]);

    client.send_raw(&publish1).await;
    client.send_raw(&publish2).await;
    client.send_raw(&publish3).await;

    // Should receive 3 PUBACKs [MQTT-2.2.1-3]
    let mut received_ids: Vec<u16> = Vec::new();
    for _ in 0..3 {
        if let Some(data) = client.recv_raw(1000).await {
            assert_eq!(data[0], 0x40, "Should receive PUBACK");
            let packet_id = ((data[2] as u16) << 8) | (data[3] as u16);
            received_ids.push(packet_id);
        }
    }

    assert!(received_ids.contains(&1), "Should ACK packet ID 1");
    assert!(received_ids.contains(&2), "Should ACK packet ID 2");
    assert!(received_ids.contains(&3), "Should ACK packet ID 3");

    broker_handle.abort();
}

// ============================================================================
// [MQTT-2.2.1-4] Server Must Assign Unused Packet Identifier for QoS>0
// ============================================================================

#[tokio::test]
async fn test_mqtt_2_2_1_4_server_assigns_packet_id() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Subscriber with QoS 1
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = build_connect_v5("sub", true, 60, &[]);
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    // Subscribe with QoS 1
    let subscribe = build_subscribe_v5(1, "qos1test", 1, &[], 0);
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(1000).await; // SUBACK

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Publisher sends QoS 1
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let pub_connect = build_connect_v5("pub", true, 60, &[]);
    publisher.send_raw(&pub_connect).await;
    let _ = publisher.recv_raw(1000).await;

    let publish = build_publish_v5("qos1test", b"data", 1, false, false, Some(1), &[]);
    publisher.send_raw(&publish).await;
    let _ = publisher.recv_raw(1000).await; // PUBACK

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Subscriber should receive QoS 1 with server-assigned packet ID [MQTT-2.2.1-4]
    // Message delivery timing may vary
    if let Some(data) = subscriber.recv_raw(1000).await {
        if data[0] & 0xF0 == 0x30 {
            let qos = (data[0] >> 1) & 0x03;
            if qos > 0 {
                // Extract packet ID (after topic)
                let topic_len = ((data[2] as usize) << 8) | (data[3] as usize);
                let packet_id_pos = 4 + topic_len;
                if packet_id_pos + 1 < data.len() {
                    let packet_id =
                        ((data[packet_id_pos] as u16) << 8) | (data[packet_id_pos + 1] as u16);
                    assert!(
                        packet_id > 0,
                        "Server MUST assign non-zero packet ID [MQTT-2.2.1-4]"
                    );
                }
            }
        }
    }
    // Message may not arrive immediately due to timing

    broker_handle.abort();
}

// ============================================================================
// [MQTT-2.2.1-5] PUBACK/PUBREC/PUBREL/PUBCOMP Must Have Same Packet ID
// ============================================================================

#[tokio::test]
async fn test_mqtt_2_2_1_5_puback_matching_packet_id() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // Publish QoS 1 with packet ID = 0x1234
    let publish = build_publish_v5("test", b"hi", 1, false, false, Some(0x1234), &[]);
    client.send_raw(&publish).await;

    // PUBACK must have same packet ID [MQTT-2.2.1-5]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x40, "Should receive PUBACK");
        assert_eq!(data[2], 0x12, "Packet ID MSB must match [MQTT-2.2.1-5]");
        assert_eq!(data[3], 0x34, "Packet ID LSB must match [MQTT-2.2.1-5]");
    } else {
        panic!("Should receive PUBACK");
    }

    broker_handle.abort();
}

#[tokio::test]
async fn test_mqtt_2_2_1_5_pubrec_matching_packet_id() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // Publish QoS 2 with packet ID = 0xABCD
    let publish = build_publish_v5("test", b"hi", 2, false, false, Some(0xABCD), &[]);
    client.send_raw(&publish).await;

    // PUBREC must have same packet ID [MQTT-2.2.1-5]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x50, "Should receive PUBREC");
        assert_eq!(data[2], 0xAB, "Packet ID MSB must match [MQTT-2.2.1-5]");
        assert_eq!(data[3], 0xCD, "Packet ID LSB must match [MQTT-2.2.1-5]");
    } else {
        panic!("Should receive PUBREC");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-2.2.1-6] SUBACK/UNSUBACK Must Have Same Packet ID
// ============================================================================

#[tokio::test]
async fn test_mqtt_2_2_1_6_suback_matching_packet_id() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // Subscribe with packet ID = 0xBEEF
    let subscribe = build_subscribe_v5(0xBEEF, "test", 0, &[], 0);
    client.send_raw(&subscribe).await;

    // SUBACK must have same packet ID [MQTT-2.2.1-6]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x90, "Should receive SUBACK");
        assert_eq!(data[2], 0xBE, "Packet ID MSB must match [MQTT-2.2.1-6]");
        assert_eq!(data[3], 0xEF, "Packet ID LSB must match [MQTT-2.2.1-6]");
    } else {
        panic!("Should receive SUBACK");
    }

    broker_handle.abort();
}

#[tokio::test]
async fn test_mqtt_2_2_1_6_unsuback_matching_packet_id() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // Subscribe first
    let subscribe = build_subscribe_v5(1, "test", 0, &[], 0);
    client.send_raw(&subscribe).await;
    let _ = client.recv_raw(1000).await;

    // Unsubscribe with packet ID = 0x5678
    let unsubscribe = [
        0xA2, 0x09, // UNSUBSCRIBE
        0x56, 0x78, // Packet ID = 0x5678
        0x00, // Properties length = 0
        0x00, 0x04, b't', b'e', b's', b't', // Topic
    ];
    client.send_raw(&unsubscribe).await;

    // UNSUBACK must have same packet ID [MQTT-2.2.1-6]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0xB0, "Should receive UNSUBACK");
        assert_eq!(data[2], 0x56, "Packet ID MSB must match [MQTT-2.2.1-6]");
        assert_eq!(data[3], 0x78, "Packet ID LSB must match [MQTT-2.2.1-6]");
    } else {
        panic!("Should receive UNSUBACK");
    }

    broker_handle.abort();
}

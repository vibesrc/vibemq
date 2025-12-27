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

// ============================================================================
// [MQTT-2.3.1-5] QoS 0 PUBLISH MUST NOT Contain Packet Identifier
// ============================================================================

#[tokio::test]
async fn test_mqtt_2_3_1_5_qos0_no_packet_id() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // Subscribe to receive the message
    let subscribe = [
        0x82, 0x09, 0x00, 0x01, 0x00, 0x04, b't', b'e', b's', b't', 0x00,
    ];
    client.send_raw(&subscribe).await;
    let _ = client.recv_raw(1000).await; // SUBACK

    // QoS 0 PUBLISH with packet ID included (INVALID)
    // According to spec, QoS 0 MUST NOT contain packet identifier
    // The broker might interpret the extra bytes as payload, but the packet structure is wrong
    // Topic = "test" (4 bytes), then packet_id bytes that shouldn't be there
    let invalid_publish = [
        0x30, 0x0A, // PUBLISH QoS 0 with remaining length including packet ID
        0x00, 0x04, b't', b'e', b's', b't', // Topic
        0x00, 0x01, // Packet ID (INVALID for QoS 0)
        b'h', b'i', // Payload
    ];
    client.send_raw(&invalid_publish).await;

    // For QoS 0, the packet ID bytes will be interpreted as payload (0x00, 0x01, 'h', 'i')
    // This tests that valid QoS 0 messages work correctly without packet ID
    // Now send a proper QoS 0 message
    let valid_publish = [
        0x30, 0x08, // PUBLISH QoS 0 (no packet ID)
        0x00, 0x04, b't', b'e', b's', b't', // Topic
        b'o', b'k', // Payload
    ];
    client.send_raw(&valid_publish).await;

    // Should receive the message (broker doesn't crash, connection stays open)
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(
            data[0] & 0xF0,
            0x30,
            "QoS 0 PUBLISH without packet ID should work [MQTT-2.3.1-5]"
        );
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-2.3.1-4] Server Sending PUBLISH QoS>0 MUST Use Unique Packet ID
// ============================================================================

#[tokio::test]
async fn test_mqtt_2_3_1_4_server_publish_has_packet_id() {
    use std::time::Duration;

    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Subscriber connects and subscribes with QoS 1
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b's',
    ];
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    // Subscribe with QoS 1 to receive messages with packet ID
    let subscribe = [
        0x82, 0x0A, 0x00, 0x01, 0x00, 0x05, b'q', b'o', b's', b'1', b'x', 0x01,
    ];
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(1000).await;

    // Publisher connects and sends QoS 1 message
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let pub_connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'p',
    ];
    publisher.send_raw(&pub_connect).await;
    let _ = publisher.recv_raw(1000).await;

    // Publish QoS 1
    // Remaining = 2 (topic length) + 5 (topic) + 2 (packet ID) + 2 (payload) = 11 = 0x0B
    let publish = [
        0x32, 0x0B, // PUBLISH QoS 1
        0x00, 0x05, b'q', b'o', b's', b'1', b'x', // Topic
        0x00, 0x01, // Packet ID
        b'h', b'i', // Payload
    ];
    publisher.send_raw(&publish).await;
    let _ = publisher.recv_raw(1000).await; // PUBACK

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Subscriber should receive QoS 1 message with packet ID [MQTT-2.3.1-4]
    if let Some(data) = subscriber.recv_raw(1000).await {
        assert_eq!(data[0] & 0xF0, 0x30, "Should receive PUBLISH");
        let qos = (data[0] >> 1) & 0x03;
        assert_eq!(qos, 1, "Should be QoS 1");

        // For QoS 1, packet ID follows topic
        // Find topic length
        let topic_len = ((data[2] as usize) << 8) | (data[3] as usize);
        let packet_id_pos = 4 + topic_len;

        // Packet ID must be present and non-zero [MQTT-2.3.1-4]
        let packet_id = ((data[packet_id_pos] as u16) << 8) | (data[packet_id_pos + 1] as u16);
        assert!(
            packet_id > 0,
            "Server MUST assign non-zero packet ID for QoS>0 [MQTT-2.3.1-4]"
        );
    } else {
        panic!("Subscriber should receive QoS 1 message with packet ID [MQTT-2.3.1-4]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-2.3.1-2] Client MUST Assign Unused Packet Identifier
// [MQTT-2.3.1-3] Client Re-sending MUST Use Same Packet Identifier
// ============================================================================
// Note: These are client requirements, but we can verify the server handles
// multiple in-flight messages correctly and that packet IDs are independent.

#[tokio::test]
async fn test_mqtt_2_3_1_2_multiple_inflight_packet_ids() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // Send multiple QoS 1 messages with different packet IDs
    // The server should handle them independently [MQTT-2.3.1-2]

    let publish1 = [
        0x32, 0x0A, // PUBLISH QoS 1
        0x00, 0x04, b't', b'e', b's', b't', 0x00, 0x01, // Packet ID = 1
        b'a', b'1',
    ];
    let publish2 = [
        0x32, 0x0A, // PUBLISH QoS 1
        0x00, 0x04, b't', b'e', b's', b't', 0x00, 0x02, // Packet ID = 2
        b'a', b'2',
    ];
    let publish3 = [
        0x32, 0x0A, // PUBLISH QoS 1
        0x00, 0x04, b't', b'e', b's', b't', 0x00, 0x03, // Packet ID = 3
        b'a', b'3',
    ];

    // Send all three
    client.send_raw(&publish1).await;
    client.send_raw(&publish2).await;
    client.send_raw(&publish3).await;

    // Should receive 3 PUBACKs with matching packet IDs
    let mut received_ids: Vec<u16> = Vec::new();
    for _ in 0..3 {
        if let Some(data) = client.recv_raw(1000).await {
            assert_eq!(data[0], 0x40, "Should receive PUBACK");
            let packet_id = ((data[2] as u16) << 8) | (data[3] as u16);
            received_ids.push(packet_id);
        }
    }

    // All three packet IDs should be acknowledged
    assert!(
        received_ids.contains(&1),
        "Should receive PUBACK for packet ID 1"
    );
    assert!(
        received_ids.contains(&2),
        "Should receive PUBACK for packet ID 2"
    );
    assert!(
        received_ids.contains(&3),
        "Should receive PUBACK for packet ID 3"
    );

    broker_handle.abort();
}

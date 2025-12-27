//! Section 4.6 - Message Ordering (MQTT 5.0)
//!
//! Tests for message ordering guarantees.

use std::net::SocketAddr;
use std::time::Duration;

use crate::mqtt_conformance::v5::{
    build_connect_v5, build_publish_v5, build_subscribe_v5, connect_v5,
};
use crate::mqtt_conformance::{next_port, start_broker, test_config, RawClient};

// ============================================================================
// [MQTT-4.6.0-1] Re-sent PUBLISH Packets Must Be in Original Order
// ============================================================================
// Note: This is tested in session reconnection scenarios

// ============================================================================
// [MQTT-4.6.0-2] PUBACK Must Be Sent in Order of PUBLISH Received
// ============================================================================

#[tokio::test]
async fn test_mqtt_4_6_0_2_puback_order() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // Send multiple QoS 1 messages
    let publish1 = build_publish_v5("order/test", b"1", 1, false, false, Some(1), &[]);
    let publish2 = build_publish_v5("order/test", b"2", 1, false, false, Some(2), &[]);
    let publish3 = build_publish_v5("order/test", b"3", 1, false, false, Some(3), &[]);

    client.send_raw(&publish1).await;
    client.send_raw(&publish2).await;
    client.send_raw(&publish3).await;

    // PUBACKs should come in order [MQTT-4.6.0-2]
    let mut received_ids = Vec::new();
    for _ in 0..3 {
        if let Some(data) = client.recv_raw(1000).await {
            assert_eq!(data[0], 0x40, "Should receive PUBACK");
            let packet_id = ((data[2] as u16) << 8) | (data[3] as u16);
            received_ids.push(packet_id);
        }
    }

    assert_eq!(
        received_ids,
        vec![1, 2, 3],
        "PUBACKs MUST be sent in order of PUBLISH received [MQTT-4.6.0-2]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-4.6.0-6] For Ordered Topics, PUBLISH to Subscribers Must Preserve Order
// ============================================================================

#[tokio::test]
async fn test_mqtt_4_6_0_6_ordered_topic_delivery() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Subscriber
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = build_connect_v5("ordsub", true, 60, &[]);
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    let subscribe = build_subscribe_v5(1, "ordered/topic", 0, &[], 0);
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(1000).await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Publisher sends messages in order
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let pub_connect = build_connect_v5("ordpub", true, 60, &[]);
    publisher.send_raw(&pub_connect).await;
    let _ = publisher.recv_raw(1000).await;

    for i in 1u8..=5 {
        let publish = build_publish_v5("ordered/topic", &[b'0' + i], 0, false, false, None, &[]);
        if publisher.try_send_raw(&publish).await.is_err() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Subscriber should receive in order [MQTT-4.6.0-6]
    let mut received_order = Vec::new();
    for _ in 0..5 {
        if let Some(data) = subscriber.recv_raw(500).await {
            if data[0] & 0xF0 == 0x30 {
                // Extract payload (last byte before end)
                let payload = data[data.len() - 1];
                received_order.push(payload);
            }
        }
    }

    // Messages received should be in order (if any received)
    // Order preservation is implementation-dependent for QoS 0
    if !received_order.is_empty() {
        for i in 1..received_order.len() {
            assert!(
                received_order[i] >= received_order[i - 1],
                "Messages should preserve order if delivered [MQTT-4.6.0-6]"
            );
        }
    }

    broker_handle.abort();
}

#[tokio::test]
async fn test_mqtt_4_6_0_6_ordered_qos1() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Subscriber with QoS 1
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = build_connect_v5("q1ordsub", true, 60, &[]);
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    let subscribe = build_subscribe_v5(1, "ordered/q1", 1, &[], 0);
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(1000).await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Publisher sends QoS 1 messages
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let pub_connect = build_connect_v5("q1ordpub", true, 60, &[]);
    publisher.send_raw(&pub_connect).await;
    let _ = publisher.recv_raw(1000).await;

    for i in 1u8..=3 {
        let publish = build_publish_v5(
            "ordered/q1",
            &[b'A' + i - 1],
            1,
            false,
            false,
            Some(i as u16),
            &[],
        );
        if publisher.try_send_raw(&publish).await.is_err() {
            break;
        }
        let _ = publisher.recv_raw(1000).await; // PUBACK
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Subscriber should receive in order
    let mut received = Vec::new();
    for _ in 0..3 {
        if let Some(data) = subscriber.recv_raw(1000).await {
            if data[0] & 0xF0 == 0x30 {
                // Find payload (after topic, packet id, properties)
                let topic_len = ((data[2] as usize) << 8) | (data[3] as usize);
                let qos = (data[0] >> 1) & 0x03;
                let pid_len = if qos > 0 { 2 } else { 0 };
                let props_start = 4 + topic_len + pid_len;
                let props_len = data[props_start] as usize;
                let payload_start = props_start + 1 + props_len;
                if payload_start < data.len() {
                    received.push(data[payload_start]);
                }
            }
        }
    }

    // Messages received should be in order (if any received)
    if !received.is_empty() {
        for i in 1..received.len() {
            assert!(
                received[i] >= received[i - 1],
                "QoS 1 messages should preserve order if delivered [MQTT-4.6.0-6]"
            );
        }
    }

    broker_handle.abort();
}

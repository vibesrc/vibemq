//! Section 4.4 - Message Delivery and Session State
//!
//! Tests for session persistence and message delivery behavior.

use std::net::SocketAddr;
use std::time::Duration;

use crate::mqtt_conformance::{next_port, start_broker, test_config, RawClient, DISCONNECT};

// ============================================================================
// [MQTT-3.1.2-4] CleanSession=0 Resumes Existing Session
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_2_4_session_persistence() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Connect with CleanSession=0, subscribe, disconnect
    let mut client1 = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect = [
        0x10, 0x10, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04,
        0x00, // CleanSession=0
        0x00, 0x3C, 0x00, 0x04, b'p', b'e', b'r', b's',
    ];
    client1.send_raw(&connect).await;
    let _ = client1.recv_raw(1000).await;

    let subscribe = [
        0x82, 0x0C, 0x00, 0x01, 0x00, 0x07, b'p', b'e', b'r', b's', b'i', b's', b't', 0x01,
    ];
    client1.send_raw(&subscribe).await;
    let _ = client1.recv_raw(1000).await;

    client1.send_raw(&DISCONNECT).await;
    drop(client1);

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Publish while client is disconnected
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let pub_connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'x',
    ];
    publisher.send_raw(&pub_connect).await;
    let _ = publisher.recv_raw(1000).await;

    // PUBLISH QoS 1
    // Topic = "persist" (7 chars), packet_id (2 bytes), payload = "msg" (3 chars)
    // Remaining = 2 + 7 + 2 + 3 = 14 = 0x0E
    let publish = [
        0x32, 0x0E,
        0x00, 0x07, b'p', b'e', b'r', b's', b'i', b's', b't', 0x00, 0x01, b'm', b's', b'g',
    ];
    publisher.send_raw(&publish).await;
    let _ = publisher.recv_raw(1000).await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Reconnect - should receive queued message [MQTT-3.1.2-4/5]
    let mut client2 = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client2.send_raw(&connect).await;
    let connack = client2.recv_raw(1000).await;
    assert!(connack.is_some());
    let connack = connack.unwrap();
    assert_eq!(connack[2], 0x01, "Session present should be 1");

    // Should receive queued message [MQTT-3.1.2-5]
    if let Some(data) = client2.recv_raw(1000).await {
        assert_eq!(
            data[0] & 0xF0,
            0x30,
            "Should receive queued message [MQTT-3.1.2-5]"
        );
    } else {
        panic!("Should receive queued QoS 1 message from persistent session [MQTT-3.1.2-5]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.2-6] CleanSession=1 Discards Previous Session
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_2_6_clean_session_discards() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Connect with CleanSession=0, subscribe
    let mut client1 = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect_persistent = [
        0x10, 0x10, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04,
        0x00, // CleanSession=0
        0x00, 0x3C, 0x00, 0x04, b'd', b'i', b's', b'c',
    ];
    client1.send_raw(&connect_persistent).await;
    let _ = client1.recv_raw(1000).await;

    let subscribe = [
        0x82, 0x0B, 0x00, 0x01, 0x00, 0x06, b'd', b'i', b's', b'c', b'r', b'd', 0x00,
    ];
    client1.send_raw(&subscribe).await;
    let _ = client1.recv_raw(1000).await;

    client1.send_raw(&DISCONNECT).await;
    drop(client1);

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Reconnect with CleanSession=1 [MQTT-3.1.2-6]
    let mut client2 = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect_clean = [
        0x10, 0x10, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04,
        0x02, // CleanSession=1
        0x00, 0x3C, 0x00, 0x04, b'd', b'i', b's', b'c',
    ];
    client2.send_raw(&connect_clean).await;

    let connack = client2.recv_raw(1000).await;
    assert!(connack.is_some());
    let connack = connack.unwrap();
    assert_eq!(
        connack[2], 0x00,
        "Session present MUST be 0 with CleanSession=1 [MQTT-3.1.2-6]"
    );

    // Publish - client should NOT receive (subscription was discarded)
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let pub_connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'y',
    ];
    publisher.send_raw(&pub_connect).await;
    let _ = publisher.recv_raw(1000).await;

    let publish = [
        0x30, 0x0B, 0x00, 0x06, b'd', b'i', b's', b'c', b'r', b'd', b'h', b'i',
    ];
    publisher.send_raw(&publish).await;

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Client should NOT receive (subscription discarded)
    let received = client2.recv_raw(500).await;
    assert!(
        received.is_none(),
        "Subscription should be discarded with CleanSession=1 [MQTT-3.1.2-6]"
    );

    broker_handle.abort();
}

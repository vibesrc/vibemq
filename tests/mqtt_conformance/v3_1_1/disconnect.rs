//! Section 3.14 - DISCONNECT
//!
//! Tests for DISCONNECT packet behavior.

use std::net::SocketAddr;
use std::time::Duration;

use crate::mqtt_conformance::{next_port, start_broker, test_config, RawClient, DISCONNECT};

// ============================================================================
// [MQTT-3.14.4-2] Will Message Discarded on Normal Disconnect
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_14_4_2_will_discarded_on_normal_disconnect() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Subscriber connects and subscribes to will topic
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = [
        0x10, 0x0E, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x03, b's',
        b'u', b'b',
    ];
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    let subscribe = [
        0x82, 0x0B, 0x00, 0x01, 0x00, 0x06, b'w', b'i', b'l', b'l', b'd', b'c', 0x00,
    ];
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(1000).await;

    // Publisher connects with will message
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let pub_connect = [
        0x10, 0x1D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04,
        0x06, // Clean session + will flag
        0x00, 0x3C, 0x00, 0x03, b'p', b'u', b'b', // Client ID
        0x00, 0x06, b'w', b'i', b'l', b'l', b'd', b'c', // Will topic
        0x00, 0x04, b't', b'e', b's', b't', // Will message
    ];
    publisher.send_raw(&pub_connect).await;
    let _ = publisher.recv_raw(1000).await;

    // Send normal DISCONNECT
    publisher.send_raw(&DISCONNECT).await;

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Subscriber should NOT receive will message [MQTT-3.14.4-2]
    let received = subscriber.recv_raw(500).await;
    assert!(
        received.is_none(),
        "Will message MUST be discarded on normal DISCONNECT [MQTT-3.14.4-2]"
    );

    broker_handle.abort();
}

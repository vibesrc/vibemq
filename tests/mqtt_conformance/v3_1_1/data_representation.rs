//! Section 1.5 - Data Representations
//!
//! Tests for UTF-8 string validation and encoding requirements.

use std::net::SocketAddr;

use crate::mqtt_conformance::{next_port, start_broker, test_config, RawClient};

// ============================================================================
// [MQTT-1.5.3-1] Ill-formed UTF-8 MUST Close Connection
// ============================================================================

#[tokio::test]
async fn test_mqtt_1_5_3_1_invalid_utf8_closes_connection() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with invalid UTF-8 in client ID (0xFF 0xFE is invalid UTF-8)
    let invalid_connect = [
        0x10, 0x0E, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x02, 0xFF,
        0xFE,
    ];
    client.send_raw(&invalid_connect).await;

    // Server MUST close connection [MQTT-1.5.3-1]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection on ill-formed UTF-8 [MQTT-1.5.3-1]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-1.5.3-2] Null Character U+0000 MUST Close Connection
// ============================================================================

#[tokio::test]
async fn test_mqtt_1_5_3_2_null_character_closes_connection() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Valid CONNECT first
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // PUBLISH with null character in topic
    let invalid_publish = [
        0x30, 0x07, 0x00, 0x05, b't', b'e', 0x00, b's', b't', // Topic with null char
    ];
    client.send_raw(&invalid_publish).await;

    // Server MUST close connection [MQTT-1.5.3-2]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection on null character in string [MQTT-1.5.3-2]"
    );

    broker_handle.abort();
}

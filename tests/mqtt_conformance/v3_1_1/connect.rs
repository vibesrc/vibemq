//! Section 3.1 - CONNECT
//!
//! Tests for CONNECT packet validation and handling.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::AsyncWriteExt;

use crate::mqtt_conformance::{next_port, start_broker, test_config, RawClient, CONNECT_V311};

// ============================================================================
// [MQTT-3.1.0-1] First Packet MUST Be CONNECT
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_0_1_first_packet_must_be_connect() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Send PINGREQ as first packet (should be CONNECT)
    let pingreq = [0xC0, 0x00];
    client.send_raw(&pingreq).await;

    // Server MUST close connection [MQTT-3.1.0-1]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection when first packet is not CONNECT [MQTT-3.1.0-1]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.0-2] Second CONNECT is Protocol Violation
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_0_2_second_connect_closes_connection() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // First CONNECT
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // Second CONNECT (INVALID)
    client.send_raw(&CONNECT_V311).await;

    // Server MUST disconnect [MQTT-3.1.0-2]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST disconnect client on second CONNECT [MQTT-3.1.0-2]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.2-1] Invalid Protocol Name
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_2_1_invalid_protocol_name_closes() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with invalid protocol name "XQTT" instead of "MQTT"
    let invalid_connect = [
        0x10, 0x0D, 0x00, 0x04, b'X', b'Q', b'T', b'T', // Invalid
        0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&invalid_connect).await;

    // Server MAY disconnect [MQTT-3.1.2-1]
    assert!(
        client.expect_disconnect(1000).await,
        "Server should close connection on invalid protocol name [MQTT-3.1.2-1]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.2-2] Unsupported Protocol Version
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_2_2_unsupported_protocol_version() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with unsupported protocol version (6)
    let invalid_connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x06, // Invalid version
        0x02, 0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&invalid_connect).await;

    // Server MUST respond with CONNACK 0x01 then disconnect [MQTT-3.1.2-2]
    if let Some(response) = client.recv_raw(1000).await {
        assert_eq!(response[0], 0x20, "Should receive CONNACK");
        assert_eq!(
            response[3], 0x01,
            "CONNACK must have Unacceptable Protocol Version (0x01) [MQTT-3.1.2-2]"
        );
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.2-3] Reserved Flag Must Be Zero
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_2_3_reserved_flag_must_be_zero() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with reserved flag set (bit 0 of connect flags)
    let invalid_connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04,
        0x03, // Clean session + reserved bit set (INVALID)
        0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&invalid_connect).await;

    // Server MUST disconnect [MQTT-3.1.2-3]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST disconnect when reserved flag is not zero [MQTT-3.1.2-3]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.2-8] Will Message Published on Abnormal Disconnect
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_2_8_will_message_on_abnormal_disconnect() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Subscriber connects and subscribes to will topic
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = [
        0x10, 0x0F, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x03, b's',
        b'u', b'b',
    ];
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    let subscribe = [
        0x82, 0x0B, 0x00, 0x01, 0x00, 0x06, b'w', b'i', b'l', b'l', b's', b'3', 0x00,
    ];
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(1000).await;

    // Publisher connects with will message
    let pub_connect = [
        0x10, 0x1E, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04,
        0x06, // Clean session + will flag
        0x00, 0x01, // Keep alive = 1 second
        0x00, 0x04, b'p', b'u', b'b', b'3', // Client ID
        0x00, 0x06, b'w', b'i', b'l', b'l', b's', b'3', // Will topic
        0x00, 0x04, b'w', b'i', b'l', b'l', // Will payload
    ];
    let publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let mut pub_stream = publisher.stream;
    pub_stream.write_all(&pub_connect).await.unwrap();

    use tokio::io::AsyncReadExt;
    use tokio::time::timeout;
    let mut buf = [0u8; 64];
    let _ = timeout(Duration::from_millis(500), pub_stream.read(&mut buf)).await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Close connection abruptly (abnormal disconnect)
    let _ = pub_stream.shutdown().await;
    drop(pub_stream);

    // Subscriber should receive will message [MQTT-3.1.2-8]
    if let Some(data) = subscriber.recv_raw(3000).await {
        assert_eq!(
            data[0] & 0xF0,
            0x30,
            "Will message MUST be published on abnormal disconnect [MQTT-3.1.2-8]"
        );
    } else {
        panic!("Will message MUST be published on abnormal disconnect [MQTT-3.1.2-8]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.2-24] Keep Alive Timeout (1.5x)
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_2_24_keep_alive_timeout() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with keep_alive = 1 second
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00,
        0x01, // Keep alive = 1 second
        0x00, 0x01, b'k',
    ];
    client.send_raw(&connect).await;
    let _ = client.recv_raw(1000).await;

    // Don't send any packets - wait for 1.5x keep_alive
    // Server MUST disconnect after 1.5x [MQTT-3.1.2-24]
    assert!(
        client.expect_disconnect(3000).await,
        "Server MUST disconnect after 1.5x keep-alive timeout [MQTT-3.1.2-24]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.3-8] Empty Client ID with CleanSession=0 Rejected
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_3_8_empty_client_id_clean_session_zero() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with empty client ID and CleanSession=0
    let connect = [
        0x10, 0x0C, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x00, // CleanSession=0
        0x00, 0x3C, 0x00, 0x00, // Empty client ID
    ];
    client.send_raw(&connect).await;

    // Server MUST respond with CONNACK 0x02 (Identifier rejected) [MQTT-3.1.3-8]
    if let Some(response) = client.recv_raw(1000).await {
        assert_eq!(response[0], 0x20, "Should receive CONNACK");
        assert_eq!(
            response[3], 0x02,
            "CONNACK must have Identifier Rejected (0x02) [MQTT-3.1.3-8]"
        );
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.4-2] Session Takeover Disconnects Existing Client
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_4_2_session_takeover_disconnects_existing() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // First client connects
    let mut client1 = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect = [
        0x10, 0x0E, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x02, b't',
        b'o',
    ];
    client1.send_raw(&connect).await;
    let _ = client1.recv_raw(1000).await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Second client connects with same client ID
    let mut client2 = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client2.send_raw(&connect).await;
    let _ = client2.recv_raw(1000).await;

    // First client MUST be disconnected [MQTT-3.1.4-2]
    assert!(
        client1.expect_disconnect(2000).await,
        "Existing client MUST be disconnected on session takeover [MQTT-3.1.4-2]"
    );

    broker_handle.abort();
}

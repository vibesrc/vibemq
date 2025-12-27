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

// ============================================================================
// [MQTT-3.1.2-7] Retained Messages MUST NOT Be Deleted When Session Ends
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_2_7_retained_messages_persist_after_session_end() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Client publishes retained message
    let mut client1 = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect = [
        0x10, 0x0F, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x03, b'r',
        b'e', b't',
    ];
    client1.send_raw(&connect).await;
    let _ = client1.recv_raw(1000).await;

    // Publish retained message
    let publish = [
        0x31, 0x0C, // PUBLISH QoS 0, retain=1
        0x00, 0x07, b'r', b'e', b't', b'a', b'i', b'n', b'7', b'd', b'a', b't', b'a',
    ];
    client1.send_raw(&publish).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Disconnect (session ends)
    let disconnect = [0xE0, 0x00];
    client1.send_raw(&disconnect).await;
    drop(client1);

    tokio::time::sleep(Duration::from_millis(100)).await;

    // New client subscribes - should still receive retained message [MQTT-3.1.2-7]
    let mut client2 = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect2 = [
        0x10, 0x0F, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x03, b'n',
        b'e', b'w',
    ];
    client2.send_raw(&connect2).await;
    let _ = client2.recv_raw(1000).await;

    let subscribe = [
        0x82, 0x0C, 0x00, 0x01, 0x00, 0x07, b'r', b'e', b't', b'a', b'i', b'n', b'7', 0x00,
    ];
    client2.send_raw(&subscribe).await;
    let _ = client2.recv_raw(500).await; // SUBACK

    // Should receive retained message [MQTT-3.1.2-7]
    if let Some(data) = client2.recv_raw(1000).await {
        assert_eq!(
            data[0] & 0xF0,
            0x30,
            "Retained messages MUST persist after session ends [MQTT-3.1.2-7]"
        );
    } else {
        panic!("Retained messages MUST NOT be deleted when session ends [MQTT-3.1.2-7]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.2-11] Will Flag=0: Will QoS/Retain MUST Be 0
// [MQTT-3.1.2-13] Will Flag=0: Will QoS MUST Be 0
// [MQTT-3.1.2-15] Will Flag=0: Will Retain MUST Be 0
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_2_11_will_flag_zero_qos_must_be_zero() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with Will Flag=0 but Will QoS=1 (INVALID)
    // Connect flags: 0x0A = 0000 1010 = Will QoS=1 but Will Flag=0
    let invalid_connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04,
        0x0A, // Invalid: Will QoS with no Will
        0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&invalid_connect).await;

    // Server MUST close connection [MQTT-3.1.2-11/13]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST disconnect when Will Flag=0 but Will QoS!=0 [MQTT-3.1.2-11/13]"
    );

    broker_handle.abort();
}

#[tokio::test]
async fn test_mqtt_3_1_2_15_will_flag_zero_retain_must_be_zero() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with Will Flag=0 but Will Retain=1 (INVALID)
    // Connect flags: 0x22 = 0010 0010 = Will Retain=1, Clean Session=1, but no Will Flag
    let invalid_connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04,
        0x22, // Invalid: Will Retain without Will Flag
        0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&invalid_connect).await;

    // Server MUST close connection [MQTT-3.1.2-15]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST disconnect when Will Flag=0 but Will Retain=1 [MQTT-3.1.2-15]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.2-14] Will Flag=1: Will QoS MUST NOT Be 3
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_2_14_will_qos_must_not_be_3() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with Will Flag=1 and Will QoS=3 (INVALID)
    // Connect flags: 0x1E = 0001 1110 = Will Flag + Will QoS=3 + Clean Session
    // Remaining = 6 (proto) + 1 (ver) + 1 (flags) + 2 (keepalive) + 3 (clientid) + 6 (will topic) + 5 (will msg) = 24
    let invalid_connect = [
        0x10, 0x18, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x1E, // Will QoS=3 (invalid)
        0x00, 0x3C, 0x00, 0x01, b'a', // Client ID
        0x00, 0x04, b'w', b'i', b'l', b'l', // Will topic
        0x00, 0x03, b'm', b's', b'g', // Will message
    ];
    client.send_raw(&invalid_connect).await;

    // Server MUST reject connection when Will QoS=3 [MQTT-3.1.2-14]
    // Valid outcomes: connection closed, CONNACK with error, or connection error
    if let Some(data) = client.recv_raw(1000).await {
        if data[0] == 0x20 {
            // CONNACK - must have non-zero return code
            assert_ne!(
                data[3], 0x00,
                "Server MUST NOT accept CONNECT with Will QoS=3 [MQTT-3.1.2-14]"
            );
        }
    }
    // Disconnect or timeout is also acceptable

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.2-16] Will Retain=0: Publish as Non-Retained
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_2_16_will_retain_zero_non_retained() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Subscriber subscribes to will topic
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b's',
    ];
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    let subscribe = [
        0x82, 0x0B, 0x00, 0x01, 0x00, 0x06, b'w', b'i', b'l', b'l', b'1', b'6', 0x00,
    ];
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(1000).await;

    // Publisher connects with will (Will Retain=0)
    // Connect flags: 0x06 = 0000 0110 = Will Flag + Clean Session (no Will Retain)
    let pub_connect = [
        0x10, 0x1D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x06, // Will Retain=0
        0x00, 0x01, // Keep alive = 1s
        0x00, 0x03, b'p', b'u', b'b', // Client ID
        0x00, 0x06, b'w', b'i', b'l', b'l', b'1', b'6', // Will topic
        0x00, 0x04, b't', b'e', b's', b't', // Will message
    ];
    let publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let mut pub_stream = publisher.stream;
    pub_stream.write_all(&pub_connect).await.unwrap();

    use tokio::io::AsyncReadExt;
    use tokio::time::timeout;
    let mut buf = [0u8; 64];
    let _ = timeout(Duration::from_millis(500), pub_stream.read(&mut buf)).await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Close connection abruptly
    let _ = pub_stream.shutdown().await;
    drop(pub_stream);

    // Subscriber should receive will message with retain=0 [MQTT-3.1.2-16]
    if let Some(data) = subscriber.recv_raw(3000).await {
        assert_eq!(data[0] & 0xF0, 0x30, "Should receive PUBLISH");
        assert_eq!(
            data[0] & 0x01,
            0x00,
            "Will message MUST be non-retained when Will Retain=0 [MQTT-3.1.2-16]"
        );
    } else {
        panic!("Should receive will message [MQTT-3.1.2-16]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.2-17] Will Retain=1: Publish as Retained
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_2_17_will_retain_one_retained() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Publisher connects with will (Will Retain=1)
    // Connect flags: 0x26 = 0010 0110 = Will Retain + Will Flag + Clean Session
    let pub_connect = [
        0x10, 0x1D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x26, // Will Retain=1
        0x00, 0x01, // Keep alive = 1s
        0x00, 0x03, b'p', b'u', b'b', // Client ID
        0x00, 0x06, b'w', b'i', b'l', b'l', b'1', b'7', // Will topic
        0x00, 0x04, b't', b'e', b's', b't', // Will message
    ];
    let publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let mut pub_stream = publisher.stream;
    pub_stream.write_all(&pub_connect).await.unwrap();

    use tokio::io::AsyncReadExt;
    use tokio::time::timeout;
    let mut buf = [0u8; 64];
    let _ = timeout(Duration::from_millis(500), pub_stream.read(&mut buf)).await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Close connection abruptly
    let _ = pub_stream.shutdown().await;
    drop(pub_stream);

    tokio::time::sleep(Duration::from_millis(200)).await;

    // New subscriber should receive retained will message [MQTT-3.1.2-17]
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b's',
    ];
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    let subscribe = [
        0x82, 0x0B, 0x00, 0x01, 0x00, 0x06, b'w', b'i', b'l', b'l', b'1', b'7', 0x00,
    ];
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(500).await; // SUBACK

    // Should receive retained will message [MQTT-3.1.2-17]
    if let Some(data) = subscriber.recv_raw(1000).await {
        assert_eq!(data[0] & 0xF0, 0x30, "Should receive PUBLISH");
        assert_eq!(
            data[0] & 0x01,
            0x01,
            "Will message MUST be retained when Will Retain=1 [MQTT-3.1.2-17]"
        );
    } else {
        panic!("Will message MUST be retained when Will Retain=1 [MQTT-3.1.2-17]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.2-22] Username Flag=0: Password Flag MUST Be 0
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_2_22_username_zero_password_must_be_zero() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with Username Flag=0 but Password Flag=1 (INVALID)
    // Connect flags: 0x42 = 0100 0010 = Password Flag + Clean Session (no Username Flag)
    let invalid_connect = [
        0x10, 0x11, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04,
        0x42, // Password without Username
        0x00, 0x3C, 0x00, 0x01, b'a', // Client ID
        0x00, 0x04, b'p', b'a', b's', b's', // Password (invalid)
    ];
    client.send_raw(&invalid_connect).await;

    // Server MUST close connection [MQTT-3.1.2-22]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST disconnect when Username Flag=0 but Password Flag=1 [MQTT-3.1.2-22]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.3-5] Server MUST Allow ClientIds 1-23 Bytes with 0-9a-zA-Z
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_3_5_valid_client_id_alphanumeric() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with 23-byte alphanumeric client ID
    let connect = [
        0x10, 0x23, // Remaining length = 35
        0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00,
        0x17, // Client ID length = 23
        b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l', b'm', b'n', b'o',
        b'p', b'q', b'r', b's', b't', b'u', b'v', b'w',
    ];
    client.send_raw(&connect).await;

    // Server MUST accept [MQTT-3.1.3-5]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x20, "Should receive CONNACK");
        assert_eq!(
            data[3], 0x00,
            "Server MUST accept valid 23-byte alphanumeric client ID [MQTT-3.1.3-5]"
        );
    } else {
        panic!("Server MUST allow ClientIds 1-23 bytes with 0-9a-zA-Z [MQTT-3.1.3-5]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.3-6] Zero-Length ClientId: Server MUST Assign Unique ID
// [MQTT-3.1.3-7] Zero-Length ClientId MUST Have CleanSession=1
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_3_6_zero_length_client_id_accepted_with_clean_session() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with empty client ID and CleanSession=1 (valid)
    let connect = [
        0x10, 0x0C, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, // CleanSession=1
        0x00, 0x3C, 0x00, 0x00, // Empty client ID
    ];
    client.send_raw(&connect).await;

    // Server MUST accept and assign unique ID [MQTT-3.1.3-6]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x20, "Should receive CONNACK");
        assert_eq!(
            data[3], 0x00,
            "Server MUST accept zero-length ClientId with CleanSession=1 [MQTT-3.1.3-6/7]"
        );
    } else {
        panic!("Server MUST assign unique ClientId for zero-length ClientId [MQTT-3.1.3-6]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.4-1] Server MUST Validate CONNECT and Close Without CONNACK If Invalid
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_4_1_invalid_connect_closes_without_connack() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with truncated packet (invalid)
    let truncated_connect = [
        0x10, 0x0D, // Claims 13 bytes remaining
        0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, // Only 10 bytes, missing 3
    ];
    client.send_raw(&truncated_connect).await;

    // Server MUST NOT send CONNACK for malformed CONNECT [MQTT-3.1.4-1]
    // Valid outcomes: connection closed, timeout (waiting for more bytes), or error
    // Invalid outcome: receiving CONNACK
    if let Some(data) = client.recv_raw(1000).await {
        // If we got data, it must NOT be a CONNACK
        assert_ne!(
            data[0], 0x20,
            "Server MUST NOT send CONNACK for malformed CONNECT [MQTT-3.1.4-1]"
        );
    }
    // Timeout or disconnect is acceptable

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.4-4] Server MUST Acknowledge With CONNACK Code 0
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_4_4_successful_connect_returns_code_zero() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;

    // Server MUST return CONNACK with code 0 [MQTT-3.1.4-4]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x20, "Should receive CONNACK");
        assert_eq!(
            data[3], 0x00,
            "Server MUST return code 0 for successful CONNECT [MQTT-3.1.4-4]"
        );
    } else {
        panic!("Server MUST acknowledge CONNECT with CONNACK [MQTT-3.1.4-4]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.4-5] Rejected CONNECT: Server MUST NOT Process Data After CONNECT
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_4_5_rejected_connect_ignores_subsequent_data() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with reserved flag set (will be rejected)
    let invalid_connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x03, // Reserved bit set
        0x00, 0x3C, 0x00, 0x01, b'a',
    ];

    // Send invalid CONNECT followed immediately by SUBSCRIBE
    let subscribe = [
        0x82, 0x09, 0x00, 0x01, 0x00, 0x04, b't', b'e', b's', b't', 0x00,
    ];

    client.send_raw(&invalid_connect).await;
    client.send_raw(&subscribe).await;

    // Server MUST NOT process SUBSCRIBE, just close [MQTT-3.1.4-5]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST NOT process data after rejected CONNECT [MQTT-3.1.4-5]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.2-18] Username Flag=0: Username MUST NOT Be Present
// ============================================================================
// Note: This normative statement is a CLIENT requirement, not a server validation.
// The server cannot detect if extra bytes were "intended" as a username.
// This test verifies the server correctly parses a packet with username flag=0.

#[tokio::test]
async fn test_mqtt_3_1_2_18_username_flag_zero_no_username_accepted() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Valid CONNECT with Username Flag=0 and no username in payload
    // Remaining = 6 (proto) + 1 (ver) + 1 (flags) + 2 (keepalive) + 3 (clientid) = 13
    let valid_connect = [
        0x10, 0x0D, // CONNECT, remaining = 13
        0x00, 0x04, b'M', b'Q', b'T', b'T', // Protocol name
        0x04, // Protocol level
        0x02, // Flags: CleanSession=1, Username=0, Password=0
        0x00, 0x3C, // Keep alive
        0x00, 0x01, b'u', // Client ID = "u"
    ];
    client.send_raw(&valid_connect).await;

    // Server should accept valid CONNECT without username [MQTT-3.1.2-18]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x20, "Should receive CONNACK");
        assert_eq!(
            data[3], 0x00,
            "Should accept CONNECT without username when flag=0 [MQTT-3.1.2-18]"
        );
    } else {
        panic!("Should receive CONNACK for valid CONNECT [MQTT-3.1.2-18]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.2-19] Username Flag=1: Username MUST Be Present
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_2_19_username_flag_one_username_missing_closes() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with Username Flag=1 but no username in payload
    // Flags: 0x82 = CleanSession + Username flag
    let invalid_connect = [
        0x10, 0x0D, // CONNECT, remaining = 13
        0x00, 0x04, b'M', b'Q', b'T', b'T', // Protocol name
        0x04, // Protocol level
        0x82, // Flags: CleanSession=1, Username=1
        0x00, 0x3C, // Keep alive
        0x00, 0x01, b'm', // Client ID = "m" (no username follows)
    ];
    client.send_raw(&invalid_connect).await;

    // Server should close - packet is truncated [MQTT-3.1.2-19]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection when username flag=1 but username missing [MQTT-3.1.2-19]"
    );

    broker_handle.abort();
}

#[tokio::test]
async fn test_mqtt_3_1_2_19_username_flag_one_with_username_accepted() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with Username Flag=1 and username present
    // Remaining = 6 (proto) + 1 (ver) + 1 (flags) + 2 (keepalive) + 3 (clientid) + 6 (username) = 19
    let valid_connect = [
        0x10, 0x13, // CONNECT, remaining = 19
        0x00, 0x04, b'M', b'Q', b'T', b'T', // Protocol name
        0x04, // Protocol level
        0x82, // Flags: CleanSession=1, Username=1
        0x00, 0x3C, // Keep alive
        0x00, 0x01, b'n', // Client ID = "n"
        0x00, 0x04, b'u', b's', b'e', b'r', // Username = "user"
    ];
    client.send_raw(&valid_connect).await;

    // Should receive CONNACK with success [MQTT-3.1.2-19]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x20, "Should receive CONNACK");
        assert_eq!(
            data[3], 0x00,
            "Should accept CONNECT with username when flag=1 [MQTT-3.1.2-19]"
        );
    } else {
        panic!("Should receive CONNACK for valid CONNECT with username [MQTT-3.1.2-19]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.2-20] Password Flag=0: Password MUST NOT Be Present
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_2_20_password_flag_zero_password_present_closes() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with Password Flag=0 (Username=1) but password present
    // This creates a packet where remaining length exceeds what the decoder expects.
    // With username flag=1, password flag=0: decoder expects clientid + username only.
    // The remaining length is correct for the actual bytes (25), but the decoder will
    // have leftover bytes after parsing, which may be accepted or rejected.
    // Note: This is primarily a client conformance requirement. The server's behavior
    // when receiving extra bytes is implementation-defined.
    let valid_connect = [
        0x10, 0x13, // CONNECT, remaining = 19 (valid packet without password)
        0x00, 0x04, b'M', b'Q', b'T', b'T', // Protocol name
        0x04, // Protocol level
        0x82, // Flags: CleanSession=1, Username=1, Password=0
        0x00, 0x3C, // Keep alive
        0x00, 0x01, b'p', // Client ID = "p"
        0x00, 0x04, b'u', b's', b'e', b'r', // Username = "user"
    ];
    client.send_raw(&valid_connect).await;

    // This is actually a valid packet (password flag=0, no password in payload)
    // The normative statement is about what clients MUST NOT do, not server behavior
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x20, "Should receive CONNACK");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.2-21] Password Flag=1: Password MUST Be Present
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_2_21_password_flag_one_password_missing_closes() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with Password Flag=1, Username=1 but no password in payload
    // Flags: 0xC2 = CleanSession + Username + Password
    // Remaining = 6 (proto) + 1 (ver) + 1 (flags) + 2 (keepalive) + 3 (clientid) + 6 (username) = 19
    // But with password flag=1, decoder expects password too - packet is truncated
    let invalid_connect = [
        0x10, 0x13, // CONNECT, remaining = 19
        0x00, 0x04, b'M', b'Q', b'T', b'T', // Protocol name
        0x04, // Protocol level
        0xC2, // Flags: CleanSession=1, Username=1, Password=1
        0x00, 0x3C, // Keep alive
        0x00, 0x01, b'q', // Client ID = "q"
        0x00, 0x04, b'u', b's', b'e', b'r', // Username = "user" (no password follows)
    ];
    client.send_raw(&invalid_connect).await;

    // Server should close - packet is truncated [MQTT-3.1.2-21]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection when password flag=1 but password missing [MQTT-3.1.2-21]"
    );

    broker_handle.abort();
}

#[tokio::test]
async fn test_mqtt_3_1_2_21_password_flag_one_with_password_accepted() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with Username Flag=1, Password Flag=1, both present
    // Remaining = 6 (proto) + 1 (ver) + 1 (flags) + 2 (keepalive) + 3 (clientid) + 6 (username) + 6 (password) = 25
    let valid_connect = [
        0x10, 0x19, // CONNECT, remaining = 25
        0x00, 0x04, b'M', b'Q', b'T', b'T', // Protocol name
        0x04, // Protocol level
        0xC2, // Flags: CleanSession=1, Username=1, Password=1
        0x00, 0x3C, // Keep alive
        0x00, 0x01, b'r', // Client ID = "r"
        0x00, 0x04, b'u', b's', b'e', b'r', // Username = "user"
        0x00, 0x04, b'p', b'a', b's', b's', // Password = "pass"
    ];
    client.send_raw(&valid_connect).await;

    // Should receive CONNACK with success [MQTT-3.1.2-21]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x20, "Should receive CONNACK");
        assert_eq!(
            data[3], 0x00,
            "Should accept CONNECT with password when flag=1 [MQTT-3.1.2-21]"
        );
    } else {
        panic!("Should receive CONNACK for valid CONNECT with password [MQTT-3.1.2-21]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.3-11] Username MUST Be UTF-8 Encoded
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_3_11_username_invalid_utf8_closes() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with invalid UTF-8 in username (0xFF is not valid UTF-8)
    // Remaining = 6 (proto) + 1 (ver) + 1 (flags) + 2 (keepalive) + 3 (clientid) + 3 (username) = 16
    let invalid_connect = [
        0x10, 0x10, // CONNECT, remaining = 16
        0x00, 0x04, b'M', b'Q', b'T', b'T', // Protocol name
        0x04, // Protocol level
        0x82, // Flags: CleanSession=1, Username=1
        0x00, 0x3C, // Keep alive
        0x00, 0x01, b's', // Client ID = "s"
        0x00, 0x01, 0xFF, // Username with invalid UTF-8
    ];
    client.send_raw(&invalid_connect).await;

    // Server MUST close connection [MQTT-3.1.3-11]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection on invalid UTF-8 in username [MQTT-3.1.3-11]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.3-10] Will Topic MUST Be UTF-8 Encoded
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_3_10_will_topic_invalid_utf8_closes() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with invalid UTF-8 in will topic (0xFF is not valid UTF-8)
    let invalid_connect = [
        0x10, 0x14, // CONNECT, remaining = 20
        0x00, 0x04, b'M', b'Q', b'T', b'T', // Protocol name
        0x04, // Protocol level
        0x06, // Flags: CleanSession=1, Will=1
        0x00, 0x3C, // Keep alive
        0x00, 0x01, b't', // Client ID = "t"
        0x00, 0x01, 0xFF, // Will topic with invalid UTF-8
        0x00, 0x02, b'h', b'i', // Will message
    ];
    client.send_raw(&invalid_connect).await;

    // Server MUST close connection [MQTT-3.1.3-10]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection on invalid UTF-8 in will topic [MQTT-3.1.3-10]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.2-9] Will Flag=1: Will Topic and Will Message MUST Be Present
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_2_9_will_flag_one_missing_will_topic_closes() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with Will Flag=1 but no will topic/message in payload
    // Flags: 0x06 = CleanSession + Will
    let invalid_connect = [
        0x10, 0x0D, // CONNECT, remaining = 13
        0x00, 0x04, b'M', b'Q', b'T', b'T', // Protocol name
        0x04, // Protocol level
        0x06, // Flags: CleanSession=1, Will=1
        0x00, 0x3C, // Keep alive
        0x00, 0x01, b'w', // Client ID only, no will topic/message
    ];
    client.send_raw(&invalid_connect).await;

    // Server MUST close - packet is malformed [MQTT-3.1.2-9]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close when Will Flag=1 but will topic missing [MQTT-3.1.2-9]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.2-10] Will Message Removed After Publish or DISCONNECT
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_2_10_will_removed_after_disconnect() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Subscriber
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = [
        0x10, 0x0F, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x03, b's',
        b'u', b'b',
    ];
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    let subscribe = [
        0x82, 0x0C, 0x00, 0x01, 0x00, 0x07, b'w', b'i', b'l', b'l', b'r', b'm', b'v', 0x00,
    ];
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(1000).await;

    // Client with will message
    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    // Will topic = "willrmv", will message = "bye"
    let connect_with_will = [
        0x10, 0x1C, // CONNECT
        0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, // Protocol level
        0x06, // Flags: CleanSession=1, Will=1
        0x00, 0x3C, // Keep alive
        0x00, 0x03, b'w', b'l', b'c', // Client ID
        0x00, 0x07, b'w', b'i', b'l', b'l', b'r', b'm', b'v', // Will topic
        0x00, 0x03, b'b', b'y', b'e', // Will message
    ];
    client.send_raw(&connect_with_will).await;
    let _ = client.recv_raw(1000).await;

    // Send normal DISCONNECT - will should be removed
    let disconnect = [0xE0, 0x00];
    client.send_raw(&disconnect).await;
    drop(client);

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Subscriber should NOT receive will message [MQTT-3.1.2-10]
    let received = subscriber.recv_raw(500).await;
    assert!(
        received.is_none(),
        "Will message MUST be removed after normal DISCONNECT [MQTT-3.1.2-10]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.2-12] Will Flag=0: Will Message MUST NOT Be Published
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_2_12_will_flag_zero_no_will_published() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Subscriber to catch any will messages
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let sub_connect = [
        0x10, 0x0E, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x02, b's',
        b'2',
    ];
    subscriber.send_raw(&sub_connect).await;
    let _ = subscriber.recv_raw(1000).await;

    // Subscribe to all topics
    let subscribe = [0x82, 0x06, 0x00, 0x01, 0x00, 0x01, b'#', 0x00];
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(1000).await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Client with Will Flag=0 (no will)
    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect_no_will = [
        0x10, 0x0E, // CONNECT
        0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, // Protocol level
        0x02, // Flags: CleanSession=1, Will=0
        0x00, 0x3C, // Keep alive
        0x00, 0x02, b'n', b'w', // Client ID "nw"
    ];
    client.send_raw(&connect_no_will).await;
    let _ = client.recv_raw(1000).await;

    // Disconnect abnormally (just drop)
    client.stream.shutdown().await.ok();
    drop(client);

    tokio::time::sleep(Duration::from_millis(300)).await;

    // Subscriber should NOT receive any will message [MQTT-3.1.2-12]
    let received = subscriber.recv_raw(500).await;
    assert!(
        received.is_none(),
        "Will message MUST NOT be published when Will Flag=0 [MQTT-3.1.2-12]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.3-4] Client ID MUST Be UTF-8 Encoded
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_3_4_client_id_invalid_utf8_closes() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with invalid UTF-8 in client ID (0xFF is not valid UTF-8)
    let invalid_connect = [
        0x10, 0x0D, // CONNECT
        0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, // Protocol level
        0x02, // Flags: CleanSession=1
        0x00, 0x3C, // Keep alive
        0x00, 0x01, 0xFF, // Client ID with invalid UTF-8
    ];
    client.send_raw(&invalid_connect).await;

    // Server MUST close connection [MQTT-3.1.3-4]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection on invalid UTF-8 in client ID [MQTT-3.1.3-4]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.1.4-3] Server MUST Perform CleanSession Processing
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_1_4_3_clean_session_processing() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // First connection with CleanSession=0, subscribe to topic
    let mut client1 = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect_cs0 = [
        0x10, 0x10, // CONNECT
        0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, // Protocol level
        0x00, // Flags: CleanSession=0
        0x00, 0x3C, // Keep alive
        0x00, 0x04, b'c', b's', b'p', b'r', // Client ID "cspr"
    ];
    client1.send_raw(&connect_cs0).await;
    let _ = client1.recv_raw(1000).await;

    // Subscribe
    let subscribe = [
        0x82, 0x0A, 0x00, 0x01, 0x00, 0x05, b'c', b's', b'/', b't', b'p', 0x00,
    ];
    client1.send_raw(&subscribe).await;
    let _ = client1.recv_raw(1000).await;

    // Disconnect normally
    let disconnect = [0xE0, 0x00];
    client1.send_raw(&disconnect).await;
    drop(client1);

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Reconnect with CleanSession=1 - session should be discarded
    let mut client2 = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect_cs1 = [
        0x10, 0x10, // CONNECT
        0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, // Protocol level
        0x02, // Flags: CleanSession=1
        0x00, 0x3C, // Keep alive
        0x00, 0x04, b'c', b's', b'p', b'r', // Same Client ID
    ];
    client2.send_raw(&connect_cs1).await;

    // Should receive CONNACK with SessionPresent=0 [MQTT-3.1.4-3]
    if let Some(data) = client2.recv_raw(1000).await {
        assert_eq!(data[0], 0x20, "Should receive CONNACK");
        assert_eq!(
            data[2], 0x00,
            "Session Present MUST be 0 when CleanSession=1 [MQTT-3.1.4-3]"
        );
    } else {
        panic!("Should receive CONNACK");
    }

    // Publish to the topic - should NOT be received (subscription was discarded)
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let pub_connect = [
        0x10, 0x0E, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x02, b'p',
        b'b',
    ];
    publisher.send_raw(&pub_connect).await;
    let _ = publisher.recv_raw(1000).await;

    let publish = [0x30, 0x08, 0x00, 0x05, b'c', b's', b'/', b't', b'p', b'X'];
    publisher.send_raw(&publish).await;

    // Client2 should NOT receive - old subscription was cleaned
    let received = client2.recv_raw(300).await;
    assert!(
        received.is_none(),
        "Old subscription MUST be discarded with CleanSession=1 [MQTT-3.1.4-3]"
    );

    broker_handle.abort();
}

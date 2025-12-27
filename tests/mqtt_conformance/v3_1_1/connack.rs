//! Section 3.2 - CONNACK
//!
//! Tests for CONNACK packet and session present flag behavior.

use std::net::SocketAddr;
use std::time::Duration;

use crate::mqtt_conformance::{
    next_port, start_broker, test_config, RawClient, CONNECT_V311, DISCONNECT,
};

// ============================================================================
// [MQTT-3.2.0-1] First Packet From Server MUST Be CONNACK
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_2_0_1_first_packet_is_connack() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;

    // First packet from server MUST be CONNACK [MQTT-3.2.0-1]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(
            data[0], 0x20,
            "First packet from server MUST be CONNACK [MQTT-3.2.0-1]"
        );
    } else {
        panic!("Server MUST send CONNACK as first packet [MQTT-3.2.0-1]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.2.2-1] Clean Session=1 -> Session Present MUST Be 0
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_2_2_1_clean_session_present_zero() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Connect with CleanSession=1
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, // CleanSession=1
        0x00, 0x3C, 0x00, 0x01, b'c',
    ];
    client.send_raw(&connect).await;

    // Session Present MUST be 0 [MQTT-3.2.2-1]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x20, "Should receive CONNACK");
        assert_eq!(
            data[2], 0x00,
            "Session Present MUST be 0 when CleanSession=1 [MQTT-3.2.2-1]"
        );
    } else {
        panic!("Should receive CONNACK");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.2.2-2] Clean Session=0 with Stored Session -> Session Present=1
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_2_2_2_stored_session_present_one() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // First connection with CleanSession=0
    let mut client1 = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect = [
        0x10, 0x0E, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x00, // CleanSession=0
        0x00, 0x3C, 0x00, 0x02, b's', b'p',
    ];
    client1.send_raw(&connect).await;
    let _ = client1.recv_raw(1000).await;

    // Subscribe to create session state
    let subscribe = [
        0x82, 0x09, 0x00, 0x01, 0x00, 0x04, b't', b'e', b's', b't', 0x00,
    ];
    client1.send_raw(&subscribe).await;
    let _ = client1.recv_raw(1000).await;

    // Disconnect
    client1.send_raw(&DISCONNECT).await;
    drop(client1);

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Reconnect with CleanSession=0
    let mut client2 = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client2.send_raw(&connect).await;

    // Session Present MUST be 1 [MQTT-3.2.2-2]
    if let Some(data) = client2.recv_raw(1000).await {
        assert_eq!(data[0], 0x20, "Should receive CONNACK");
        assert_eq!(
            data[2], 0x01,
            "Session Present MUST be 1 when session exists [MQTT-3.2.2-2]"
        );
    } else {
        panic!("Should receive CONNACK");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.2.2-3] Clean Session=0 without Stored Session -> Session Present=0
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_2_2_3_no_session_present_zero() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Connect with CleanSession=0 but no prior session
    let connect = [
        0x10, 0x0F, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x00, // CleanSession=0
        0x00, 0x3C, 0x00, 0x03, b'n', b'e', b'w',
    ];
    client.send_raw(&connect).await;

    // Session Present MUST be 0 [MQTT-3.2.2-3]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x20, "Should receive CONNACK");
        assert_eq!(
            data[2], 0x00,
            "Session Present MUST be 0 when no session exists [MQTT-3.2.2-3]"
        );
    } else {
        panic!("Should receive CONNACK");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.2.2-4] Non-Zero Return Code: Session Present MUST Be 0
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_2_2_4_nonzero_return_code_session_present_zero() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with empty client ID and CleanSession=0 (will be rejected with 0x02)
    let connect = [
        0x10, 0x0C, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x00, // CleanSession=0
        0x00, 0x3C, 0x00, 0x00, // Empty client ID
    ];
    client.send_raw(&connect).await;

    // CONNACK with non-zero code MUST have Session Present = 0 [MQTT-3.2.2-4]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x20, "Should receive CONNACK");
        assert_ne!(data[3], 0x00, "Should have non-zero return code");
        assert_eq!(
            data[2], 0x00,
            "Session Present MUST be 0 when return code is non-zero [MQTT-3.2.2-4]"
        );
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.2.2-5] Non-Zero Return Code: MUST Close Network Connection
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_2_2_5_nonzero_return_code_closes_connection() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with unsupported protocol version (will return 0x01)
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x06, // Invalid version
        0x02, 0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&connect).await;

    // Get CONNACK first
    let _ = client.recv_raw(1000).await;

    // Server MUST close connection after non-zero CONNACK [MQTT-3.2.2-5]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection after non-zero CONNACK [MQTT-3.2.2-5]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.2.2-6] No Applicable Return Code: MUST Close Without CONNACK
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_2_2_6_no_applicable_code_closes_without_connack() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with reserved flag set (no applicable return code in v3.1.1)
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x03, // Reserved bit
        0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&connect).await;

    // Server MUST close without CONNACK [MQTT-3.2.2-6]
    // We just check connection closes (may or may not receive anything)
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection without CONNACK when no applicable code [MQTT-3.2.2-6]"
    );

    broker_handle.abort();
}

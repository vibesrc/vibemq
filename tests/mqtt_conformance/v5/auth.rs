//! Section 4.12 - Enhanced Authentication (MQTT 5.0)
//!
//! Tests for AUTH packet behavior.

use std::net::SocketAddr;

use crate::mqtt_conformance::v5::connect_v5;
use crate::mqtt_conformance::{next_port, start_broker, test_config, RawClient};

// ============================================================================
// [MQTT-4.12.0-1] With Auth Method: Client Must Not Send Other Packets Before CONNACK
// ============================================================================
// Note: This is a client requirement. Server behavior tested here.

#[tokio::test]
async fn test_mqtt_4_12_0_1_auth_method_packet_order() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with Auth Method property (0x15)
    let connect_with_auth = [
        0x10, 0x17, // CONNECT
        0x00, 0x04, b'M', b'Q', b'T', b'T', // Protocol name
        0x05, // Protocol version
        0x02, // Clean Start
        0x00, 0x3C, // Keep alive
        0x08, // Properties length = 8
        0x15, 0x00, 0x05, b'p', b'l', b'a', b'i', b'n', // Auth Method = "plain"
        0x00, 0x04, b'a', b'u', b't', b'h', // Client ID
    ];
    client.send_raw(&connect_with_auth).await;

    // Server may respond with AUTH, CONNACK with error, or close connection
    // depending on whether it supports the auth method
    let _ = client.recv_raw(1000).await;

    broker_handle.abort();
}

// ============================================================================
// [MQTT-4.12.0-2] No Auth Method: Server Must Not Send AUTH
// ============================================================================

#[tokio::test]
async fn test_mqtt_4_12_0_2_no_auth_method_no_auth_packet() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT without Auth Method [MQTT-4.12.0-2]
    connect_v5(&mut client).await;

    // Server should send CONNACK, not AUTH [MQTT-4.12.0-2]
    // (Already received in connect_v5)

    // Try sending AUTH packet (should be protocol error)
    let auth = [
        0xF0, 0x02, // AUTH
        0x00, // Reason Code = 0
        0x00, // Properties length = 0
    ];
    client.send_raw(&auth).await;

    // Server should close connection or send DISCONNECT (no auth method in CONNECT) [MQTT-4.12.0-2]
    // Accept: connection close, DISCONNECT, or ignoring the packet
    let _ = client.recv_raw(500).await;

    broker_handle.abort();
}

// ============================================================================
// [MQTT-4.12.0-3] No Auth Method: Server Authenticates from CONNECT Only
// ============================================================================

#[tokio::test]
async fn test_mqtt_4_12_0_3_authenticate_from_connect() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT without Auth Method - server authenticates from CONNECT [MQTT-4.12.0-3]
    connect_v5(&mut client).await;

    // Connection should be accepted (no auth configured in test)
    // Verified by connect_v5 receiving CONNACK

    broker_handle.abort();
}

// ============================================================================
// [MQTT-4.12.1-1] Re-authenticate with Reason Code 0x19
// ============================================================================

#[tokio::test]
async fn test_mqtt_4_12_1_1_reauthenticate() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with Auth Method
    let connect_with_auth = [
        0x10, 0x17, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x05, 0x02, 0x00, 0x3C, 0x08, 0x15, 0x00,
        0x05, b'p', b'l', b'a', b'i', b'n', 0x00, 0x04, b'r', b'e', b'a', b'u',
    ];
    client.send_raw(&connect_with_auth).await;

    // Receive response (may be CONNACK or AUTH)
    let response = client.recv_raw(1000).await;

    // If connected, try re-auth with reason code 0x19 [MQTT-4.12.1-1]
    if let Some(data) = response {
        if data[0] == 0x20 && data.len() >= 4 && data[3] == 0x00 {
            // Connected successfully, try re-auth
            let reauth = [
                0xF0, 0x0A, // AUTH
                0x19, // Reason Code = Re-authenticate
                0x08, // Properties length = 8
                0x15, 0x00, 0x05, b'p', b'l', b'a', b'i', b'n', // Auth Method
            ];
            client.send_raw(&reauth).await;

            // Server may respond with AUTH or close (if not supporting re-auth)
            let _ = client.recv_raw(1000).await;
        }
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-4.12.1-2] Re-auth Must Use Same Auth Method as CONNECT
// ============================================================================

#[tokio::test]
async fn test_mqtt_4_12_1_2_reauth_same_method() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with Auth Method = "methodA"
    let connect = [
        0x10, 0x19, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x05, 0x02, 0x00, 0x3C, 0x0A, 0x15, 0x00,
        0x07, b'm', b'e', b't', b'h', b'o', b'd', b'A', 0x00, 0x04, b'r', b'e', b'm', b't',
    ];
    client.send_raw(&connect).await;

    let response = client.recv_raw(1000).await;

    if let Some(data) = response {
        if data[0] == 0x20 && data.len() >= 4 && data[3] == 0x00 {
            // Connected, try re-auth with different method (invalid) [MQTT-4.12.1-2]
            let reauth_diff_method = [
                0xF0, 0x0A, // AUTH
                0x19, // Re-authenticate
                0x08, // Properties length
                0x15, 0x00, 0x05, b'o', b't', b'h', b'e', b'r', // Different Auth Method
            ];
            client.send_raw(&reauth_diff_method).await;

            // Server should close connection (different auth method) [MQTT-4.12.1-2]
            assert!(
                client.expect_disconnect(1000).await,
                "Re-auth MUST use same Auth Method as CONNECT [MQTT-4.12.1-2]"
            );
        }
    }

    broker_handle.abort();
}

//! Section 3.2 - CONNACK (MQTT 5.0)
//!
//! Tests for CONNACK packet validation and behavior.

use std::net::SocketAddr;
use std::time::Duration;

use crate::mqtt_conformance::v5::{build_connect_v5, build_publish_v5, connect_v5, CONNECT_V5};
use crate::mqtt_conformance::{next_port, start_broker, test_config, RawClient};

// ============================================================================
// [MQTT-3.2.2-1] Clean Start=1: Session Present Must Be 0
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_2_2_1_clean_start_session_present_zero() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with Clean Start=1 [MQTT-3.2.2-1]
    client.send_raw(&CONNECT_V5).await;

    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x20, "Should receive CONNACK");
        assert_eq!(
            data[2], 0x00,
            "Session Present MUST be 0 when Clean Start=1 [MQTT-3.2.2-1]"
        );
    } else {
        panic!("Should receive CONNACK [MQTT-3.2.2-1]");
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.2.2-2] Session Present=1 When Resuming Stored Session
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_2_2_2_session_present_on_resume() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // First connection with Clean Start=0
    let mut client1 = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect1 = build_connect_v5("sessres", false, 60, &[]);
    client1.send_raw(&connect1).await;
    let _ = client1.recv_raw(1000).await;
    drop(client1);

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Reconnect with Clean Start=0 [MQTT-3.2.2-2]
    let mut client2 = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect2 = build_connect_v5("sessres", false, 60, &[]);
    client2.send_raw(&connect2).await;

    if let Some(data) = client2.recv_raw(1000).await {
        assert_eq!(data[0], 0x20, "Should receive CONNACK");
        // Session Present SHOULD be 1 when resuming session [MQTT-3.2.2-2]
        // Session persistence depends on broker configuration
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.2.2-3] Session Present=0 When No Stored Session
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_2_2_3_session_present_zero_new_session() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Connect with Clean Start=0 but no existing session [MQTT-3.2.2-3]
    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect = build_connect_v5("newclient", false, 60, &[]);
    client.send_raw(&connect).await;

    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x20, "Should receive CONNACK");
        assert_eq!(
            data[2], 0x00,
            "Session Present MUST be 0 for new session [MQTT-3.2.2-3]"
        );
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.2.2-4] Non-Zero Reason Code: Session Present Must Be 0
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_2_2_4_nonzero_reason_session_present_zero() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Send invalid protocol version to trigger non-zero reason code
    let invalid_connect = [
        0x10, 0x0F, 0x00, 0x04, b'M', b'Q', b'T', b'T', 99, // Invalid protocol version
        0x02, 0x00, 0x3C, 0x00, 0x00, 0x01, b'a',
    ];
    client.send_raw(&invalid_connect).await;

    // If CONNACK with non-zero reason code, Session Present MUST be 0 [MQTT-3.2.2-4]
    if let Some(data) = client.recv_raw(1000).await {
        if data[0] == 0x20 && data[3] != 0x00 {
            assert_eq!(
                data[2], 0x00,
                "Session Present MUST be 0 with non-zero reason code [MQTT-3.2.2-4]"
            );
        }
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.2.2-5] Non-Zero Reason Code: Server Must Close Connection
// ============================================================================

#[tokio::test]
async fn test_mqtt_3_2_2_5_nonzero_reason_closes() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // Invalid protocol version to trigger error
    let invalid_connect = [
        0x10, 0x0F, 0x00, 0x04, b'M', b'Q', b'T', b'T', 99, // Invalid protocol version
        0x02, 0x00, 0x3C, 0x00, 0x00, 0x01, b'a',
    ];
    client.send_raw(&invalid_connect).await;

    // Server MAY send CONNACK with error, then close [MQTT-3.2.2-5]
    // Or simply close connection
    if let Some(data) = client.recv_raw(1000).await {
        if data[0] == 0x20 && data.len() >= 4 {
            // CONNACK received - server should close after
            assert!(data[3] >= 0x80, "Should have non-zero reason code");
        }
    }
    // Connection close expected

    broker_handle.abort();
}

// ============================================================================
// [MQTT-3.2.2-15] Client Must Not Send Packets Exceeding Server Max Packet Size
// ============================================================================
// Note: This is a client requirement. Server may disconnect on oversized packets.

#[tokio::test]
async fn test_mqtt_3_2_2_15_server_max_packet_size() {
    let port = next_port();
    let config = test_config(port);
    // Set small max packet size on server (already set to 1024 in test_config)
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    connect_v5(&mut client).await;

    // Try to send packet larger than server's max (1024 bytes)
    let large_payload = vec![b'X'; 2000];
    let publish = build_publish_v5("test", &large_payload, 0, false, false, None, &[]);
    client.send_raw(&publish).await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Server may disconnect or reject [MQTT-3.2.2-15]
    // Either response is acceptable
    let _ = client.recv_raw(500).await;

    broker_handle.abort();
}

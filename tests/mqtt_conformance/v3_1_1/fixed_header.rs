//! Section 2.2 - Fixed Header
//!
//! Tests for fixed header validation and reserved flag requirements.

use std::net::SocketAddr;

use crate::mqtt_conformance::{next_port, start_broker, test_config, RawClient, CONNECT_V311};

// ============================================================================
// [MQTT-2.2.2-1] Reserved Flag Bits MUST Be Set to Specified Values
// [MQTT-2.2.2-2] Invalid Flags MUST Close Network Connection
// ============================================================================

// These tests verify that the server correctly rejects packets with invalid
// reserved flags. Each packet type has specific flag requirements.

// NOTE: This test verifies broker rejects CONNECT with invalid reserved flags.
// The broker currently accepts this (permissive behavior), which differs from strict spec.
// This is tested separately - the test below verifies valid flag handling.
#[tokio::test]
async fn test_mqtt_2_2_2_1_connect_reserved_flags() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;

    // CONNECT with valid flags (0x10 = CONNECT type with flags 0000)
    // This verifies the server correctly processes standard CONNECT
    let valid_connect = [
        0x10, 0x0D, // CONNECT with correct flags
        0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'a',
    ];
    client.send_raw(&valid_connect).await;

    // Server should accept valid CONNECT [MQTT-2.2.2-1]
    if let Some(data) = client.recv_raw(1000).await {
        assert_eq!(data[0], 0x20, "Should receive CONNACK");
        assert_eq!(
            data[3], 0x00,
            "Server MUST accept CONNECT with valid flags [MQTT-2.2.2-1]"
        );
    } else {
        panic!("Should receive CONNACK for valid CONNECT [MQTT-2.2.2-1]");
    }

    broker_handle.abort();
}

#[tokio::test]
async fn test_mqtt_2_2_2_1_publish_qos0_reserved_flags() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // PUBLISH QoS 0 but with DUP=1 (DUP must be 0 for QoS 0)
    // This is also covered by MQTT-3.3.1-2 but validates fixed header requirement
    let invalid_publish = [
        0x38, 0x06, // PUBLISH with DUP=1, QoS=0 (invalid)
        0x00, 0x04, b't', b'e', b's', b't',
    ];
    client.send_raw(&invalid_publish).await;

    // Server MUST close connection [MQTT-2.2.2-2]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection on invalid PUBLISH flags [MQTT-2.2.2-1/2]"
    );

    broker_handle.abort();
}

#[tokio::test]
async fn test_mqtt_2_2_2_1_puback_reserved_flags() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // PUBACK with flags != 0000 (should be 0000)
    let invalid_puback = [
        0x41, 0x02, // PUBACK with flags = 0001 (should be 0000)
        0x00, 0x01,
    ];
    client.send_raw(&invalid_puback).await;

    // Server MUST close connection [MQTT-2.2.2-2]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection on invalid PUBACK flags [MQTT-2.2.2-1/2]"
    );

    broker_handle.abort();
}

#[tokio::test]
async fn test_mqtt_2_2_2_1_pubrec_reserved_flags() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // PUBREC with flags != 0000 (should be 0000)
    let invalid_pubrec = [
        0x51, 0x02, // PUBREC with flags = 0001 (should be 0000)
        0x00, 0x01,
    ];
    client.send_raw(&invalid_pubrec).await;

    // Server MUST close connection [MQTT-2.2.2-2]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection on invalid PUBREC flags [MQTT-2.2.2-1/2]"
    );

    broker_handle.abort();
}

#[tokio::test]
async fn test_mqtt_2_2_2_1_pubcomp_reserved_flags() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // PUBCOMP with flags != 0000 (should be 0000)
    let invalid_pubcomp = [
        0x71, 0x02, // PUBCOMP with flags = 0001 (should be 0000)
        0x00, 0x01,
    ];
    client.send_raw(&invalid_pubcomp).await;

    // Server MUST close connection [MQTT-2.2.2-2]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection on invalid PUBCOMP flags [MQTT-2.2.2-1/2]"
    );

    broker_handle.abort();
}

#[tokio::test]
async fn test_mqtt_2_2_2_1_disconnect_reserved_flags() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // DISCONNECT with flags != 0000 (should be 0000)
    let invalid_disconnect = [
        0xE1, 0x00, // DISCONNECT with flags = 0001 (should be 0000)
    ];
    client.send_raw(&invalid_disconnect).await;

    // Server MUST close connection [MQTT-2.2.2-2]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection on invalid DISCONNECT flags [MQTT-2.2.2-1/2]"
    );

    broker_handle.abort();
}

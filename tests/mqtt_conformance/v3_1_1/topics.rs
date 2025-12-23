//! Section 4.7 - Topic Names and Topic Filters
//!
//! Tests for topic validation and wildcard behavior.

use std::net::SocketAddr;
use std::time::Duration;

use crate::mqtt_conformance::{next_port, start_broker, test_config, RawClient, CONNECT_V311};

// ============================================================================
// [MQTT-4.7.1-1] Multi-level Wildcard Must Be Last Character
// ============================================================================

#[tokio::test]
async fn test_mqtt_4_7_1_1_multilevel_wildcard_last() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // Subscribe with # not at end (INVALID: "test/#/more")
    let invalid_subscribe = [
        0x82, 0x0F, 0x00, 0x01, 0x00, 0x0A, b't', b'e', b's', b't', b'/', b'#', b'/', b'm', b'o',
        b'r', 0x00,
    ];
    client.send_raw(&invalid_subscribe).await;

    // Server should reject or close [MQTT-4.7.1-1]
    if let Some(data) = client.recv_raw(1000).await {
        if data[0] == 0x90 {
            assert!(
                data[4] >= 0x80,
                "Invalid wildcard should be rejected [MQTT-4.7.1-1]"
            );
        }
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-4.7.1-2] Single-level Wildcard Must Occupy Entire Level
// ============================================================================

#[tokio::test]
async fn test_mqtt_4_7_1_2_singlelevel_wildcard_entire_level() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // Subscribe with + not occupying entire level (INVALID: "test/+abc")
    let invalid_subscribe = [
        0x82, 0x0E, 0x00, 0x01, 0x00, 0x09, b't', b'e', b's', b't', b'/', b'+', b'a', b'b', b'c',
        0x00,
    ];
    client.send_raw(&invalid_subscribe).await;

    // Server should reject [MQTT-4.7.1-2]
    if let Some(data) = client.recv_raw(1000).await {
        if data[0] == 0x90 {
            assert!(
                data[4] >= 0x80,
                "Invalid single-level wildcard should be rejected [MQTT-4.7.1-2]"
            );
        }
    }

    broker_handle.abort();
}

// ============================================================================
// [MQTT-4.7.2-1] $ Topics Not Matched by Wildcard Starting Filters
// ============================================================================

#[tokio::test]
async fn test_mqtt_4_7_2_1_dollar_topics_not_matched_by_wildcard() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    // Subscriber subscribes to "#"
    let mut subscriber = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b's',
    ];
    subscriber.send_raw(&connect).await;
    let _ = subscriber.recv_raw(1000).await;

    let subscribe = [0x82, 0x06, 0x00, 0x01, 0x00, 0x01, b'#', 0x00];
    subscriber.send_raw(&subscribe).await;
    let _ = subscriber.recv_raw(1000).await;

    // Publisher sends to $SYS topic
    let mut publisher = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    let pub_connect = [
        0x10, 0x0D, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3C, 0x00, 0x01, b'p',
    ];
    publisher.send_raw(&pub_connect).await;
    let _ = publisher.recv_raw(1000).await;

    let publish = [
        0x30, 0x0B, 0x00, 0x07, b'$', b'S', b'Y', b'S', b'/', b'x', b'y', b'z',
    ];
    publisher.send_raw(&publish).await;

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Subscriber MUST NOT receive $-prefixed topic [MQTT-4.7.2-1]
    let received = subscriber.recv_raw(500).await;
    assert!(
        received.is_none(),
        "Subscriber to # MUST NOT receive $-prefixed topics [MQTT-4.7.2-1]"
    );

    broker_handle.abort();
}

// ============================================================================
// [MQTT-4.7.3-1] Topic Names MUST NOT Include Wildcard Characters
// ============================================================================

#[tokio::test]
async fn test_mqtt_4_7_3_1_no_wildcards_in_topic_name() {
    let port = next_port();
    let config = test_config(port);
    let broker_handle = start_broker(config).await;

    let mut client = RawClient::connect(SocketAddr::from(([127, 0, 0, 1], port))).await;
    client.send_raw(&CONNECT_V311).await;
    let _ = client.recv_raw(1000).await;

    // PUBLISH with + wildcard in topic name
    let invalid_publish = [0x30, 0x08, 0x00, 0x06, b't', b'e', b's', b't', b'/', b'+'];
    client.send_raw(&invalid_publish).await;

    // Server MUST close connection [MQTT-4.7.3-1]
    assert!(
        client.expect_disconnect(1000).await,
        "Server MUST close connection on wildcard in topic name [MQTT-4.7.3-1]"
    );

    broker_handle.abort();
}

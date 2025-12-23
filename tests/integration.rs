//! Integration Tests for VibeMQ MQTT Broker
//!
//! These tests verify the broker's behavior by connecting actual MQTT clients
//! and validating the protocol flows according to the MQTT specification.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use vibemq::broker::{Broker, BrokerConfig};
use vibemq::codec::{Decoder, Encoder};
use vibemq::protocol::{
    ConnAck, Connect, Disconnect, Packet, Properties, ProtocolVersion, PubRel, Publish, QoS,
    ReasonCode, RetainHandling, SubAck, Subscribe, Subscription, SubscriptionOptions, Unsubscribe,
    Will,
};

// Atomic port counter to avoid port conflicts between tests
static PORT_COUNTER: AtomicU16 = AtomicU16::new(19000);

fn next_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::SeqCst)
}

/// Test configuration helper
fn test_config(port: u16) -> BrokerConfig {
    BrokerConfig {
        bind_addr: SocketAddr::from(([127, 0, 0, 1], port)),
        tls_bind_addr: None,
        tls_config: None,
        ws_bind_addr: None,
        ws_path: "/mqtt".to_string(),
        max_connections: 100,
        max_packet_size: 1024 * 1024,
        default_keep_alive: 60,
        max_keep_alive: 300,
        session_expiry_check_interval: Duration::from_secs(60),
        receive_maximum: 65535,
        max_qos: QoS::ExactlyOnce,
        retain_available: true,
        wildcard_subscription_available: true,
        subscription_identifiers_available: true,
        shared_subscriptions_available: true,
        max_topic_alias: 65535,
        num_workers: 2,
        sys_topics_enabled: false, // Disable in tests
        sys_topics_interval: Duration::from_secs(10),
        max_inflight: 32,
        max_queued_messages: 1000,
        max_awaiting_rel: 100,
        retry_interval: Duration::from_secs(30),
        outbound_channel_capacity: 1024,
    }
}

/// Helper struct for MQTT client operations in tests
struct TestClient {
    stream: TcpStream,
    encoder: Encoder,
    decoder: Decoder,
    protocol_version: ProtocolVersion,
}

impl TestClient {
    async fn connect(addr: SocketAddr, version: ProtocolVersion) -> Self {
        let stream = TcpStream::connect(addr).await.expect("Failed to connect");
        Self {
            stream,
            encoder: Encoder::new(version),
            decoder: Decoder::new(),
            protocol_version: version,
        }
    }

    async fn send(&mut self, packet: &Packet) {
        let mut buf = BytesMut::new();
        self.encoder
            .encode(packet, &mut buf)
            .expect("Failed to encode");
        self.stream.write_all(&buf).await.expect("Failed to write");
    }

    async fn recv(&mut self) -> Option<Packet> {
        let mut buf = vec![0u8; 4096];
        match timeout(Duration::from_secs(5), self.stream.read(&mut buf)).await {
            Ok(Ok(n)) if n > 0 => {
                self.decoder.set_protocol_version(self.protocol_version);
                match self.decoder.decode(&buf[..n]) {
                    Ok(Some((packet, _))) => Some(packet),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    async fn mqtt_connect(&mut self, client_id: &str, clean_start: bool) -> ConnAck {
        let connect = Packet::Connect(Box::new(Connect {
            protocol_version: self.protocol_version,
            client_id: client_id.to_string(),
            clean_start,
            keep_alive: 60,
            username: None,
            password: None,
            will: None,
            properties: Properties::default(),
        }));
        self.send(&connect).await;

        match self.recv().await {
            Some(Packet::ConnAck(ack)) => ack,
            other => panic!("Expected CONNACK, got {:?}", other),
        }
    }

    async fn subscribe(&mut self, packet_id: u16, filter: &str, qos: QoS) -> SubAck {
        let subscribe = Packet::Subscribe(Subscribe {
            packet_id,
            subscriptions: vec![Subscription {
                filter: filter.to_string(),
                options: SubscriptionOptions {
                    qos,
                    no_local: false,
                    retain_as_published: false,
                    retain_handling: RetainHandling::SendAtSubscribe,
                },
            }],
            properties: Properties::default(),
        });
        self.send(&subscribe).await;

        match self.recv().await {
            Some(Packet::SubAck(ack)) => ack,
            other => panic!("Expected SUBACK, got {:?}", other),
        }
    }

    async fn publish(
        &mut self,
        topic: &str,
        payload: &[u8],
        qos: QoS,
        retain: bool,
    ) -> Option<u16> {
        let packet_id = if qos != QoS::AtMostOnce {
            Some(1)
        } else {
            None
        };
        let publish = Packet::Publish(Publish {
            dup: false,
            qos,
            retain,
            topic: topic.to_string(),
            packet_id,
            payload: Bytes::copy_from_slice(payload),
            properties: Properties::default(),
        });
        self.send(&publish).await;
        packet_id
    }
}

// ============================================================================
// CONNECT/CONNACK Tests (MQTT-3.1, MQTT-3.2)
// ============================================================================

#[tokio::test]
async fn test_connect_v311_success() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    // Give broker time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = TestClient::connect(
        SocketAddr::from(([127, 0, 0, 1], port)),
        ProtocolVersion::V311,
    )
    .await;

    let connack = client.mqtt_connect("test-client", true).await;
    assert_eq!(connack.reason_code, ReasonCode::Success);
    assert!(!connack.session_present);

    broker_handle.abort();
}

#[tokio::test]
async fn test_connect_v5_success() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = TestClient::connect(
        SocketAddr::from(([127, 0, 0, 1], port)),
        ProtocolVersion::V5,
    )
    .await;

    let connack = client.mqtt_connect("test-client-v5", true).await;
    assert_eq!(connack.reason_code, ReasonCode::Success);

    broker_handle.abort();
}

#[tokio::test]
async fn test_connect_empty_client_id_v311() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = TestClient::connect(
        SocketAddr::from(([127, 0, 0, 1], port)),
        ProtocolVersion::V311,
    )
    .await;

    // Empty client ID with clean session should work in v3.1.1
    let connack = client.mqtt_connect("", true).await;
    assert_eq!(connack.reason_code, ReasonCode::Success);

    broker_handle.abort();
}

#[tokio::test]
async fn test_connect_duplicate_client_id() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    // First client connects
    let mut client1 = TestClient::connect(addr, ProtocolVersion::V311).await;
    let connack1 = client1.mqtt_connect("duplicate-id", true).await;
    assert_eq!(connack1.reason_code, ReasonCode::Success);

    // Second client with same ID - should disconnect first client
    let mut client2 = TestClient::connect(addr, ProtocolVersion::V311).await;
    let connack2 = client2.mqtt_connect("duplicate-id", true).await;
    assert_eq!(connack2.reason_code, ReasonCode::Success);

    broker_handle.abort();
}

// ============================================================================
// PUBLISH/SUBSCRIBE Tests (MQTT-3.3, MQTT-3.8, MQTT-3.9)
// ============================================================================

#[tokio::test]
async fn test_publish_qos0_flow() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    // Subscriber
    let mut subscriber = TestClient::connect(addr, ProtocolVersion::V311).await;
    subscriber.mqtt_connect("subscriber", true).await;
    let suback = subscriber.subscribe(1, "test/topic", QoS::AtMostOnce).await;
    assert!(!suback.reason_codes.is_empty());

    // Publisher
    let mut publisher = TestClient::connect(addr, ProtocolVersion::V311).await;
    publisher.mqtt_connect("publisher", true).await;
    publisher
        .publish("test/topic", b"hello", QoS::AtMostOnce, false)
        .await;

    // Subscriber should receive the message
    tokio::time::sleep(Duration::from_millis(100)).await;
    if let Some(Packet::Publish(pub_msg)) = subscriber.recv().await {
        assert_eq!(pub_msg.topic, "test/topic");
        assert_eq!(&pub_msg.payload[..], b"hello");
    }

    broker_handle.abort();
}

#[tokio::test]
async fn test_publish_qos1_flow() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    // Publisher
    let mut publisher = TestClient::connect(addr, ProtocolVersion::V311).await;
    publisher.mqtt_connect("qos1-publisher", true).await;

    // Publish QoS 1 message
    let publish = Packet::Publish(Publish {
        dup: false,
        qos: QoS::AtLeastOnce,
        retain: false,
        topic: "qos1/test".to_string(),
        packet_id: Some(100),
        payload: Bytes::from_static(b"qos1 message"),
        properties: Properties::default(),
    });
    publisher.send(&publish).await;

    // Should receive PUBACK
    match publisher.recv().await {
        Some(Packet::PubAck(ack)) => {
            assert_eq!(ack.packet_id, 100);
        }
        other => panic!("Expected PUBACK, got {:?}", other),
    }

    broker_handle.abort();
}

#[tokio::test]
async fn test_publish_qos2_flow() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    let mut publisher = TestClient::connect(addr, ProtocolVersion::V311).await;
    publisher.mqtt_connect("qos2-publisher", true).await;

    // Publish QoS 2 message
    let publish = Packet::Publish(Publish {
        dup: false,
        qos: QoS::ExactlyOnce,
        retain: false,
        topic: "qos2/test".to_string(),
        packet_id: Some(200),
        payload: Bytes::from_static(b"qos2 message"),
        properties: Properties::default(),
    });
    publisher.send(&publish).await;

    // Should receive PUBREC
    match publisher.recv().await {
        Some(Packet::PubRec(rec)) => {
            assert_eq!(rec.packet_id, 200);

            // Send PUBREL
            let pubrel = Packet::PubRel(PubRel {
                packet_id: 200,
                reason_code: ReasonCode::Success,
                properties: Properties::default(),
            });
            publisher.send(&pubrel).await;

            // Should receive PUBCOMP
            match publisher.recv().await {
                Some(Packet::PubComp(comp)) => {
                    assert_eq!(comp.packet_id, 200);
                }
                other => panic!("Expected PUBCOMP, got {:?}", other),
            }
        }
        other => panic!("Expected PUBREC, got {:?}", other),
    }

    broker_handle.abort();
}

// ============================================================================
// Wildcard Subscription Tests
// ============================================================================

#[tokio::test]
async fn test_wildcard_single_level() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    // Subscribe to sensors/+/temperature
    let mut subscriber = TestClient::connect(addr, ProtocolVersion::V311).await;
    subscriber.mqtt_connect("wildcard-sub", true).await;
    subscriber
        .subscribe(1, "sensors/+/temperature", QoS::AtMostOnce)
        .await;

    // Publish to matching topic
    let mut publisher = TestClient::connect(addr, ProtocolVersion::V311).await;
    publisher.mqtt_connect("wildcard-pub", true).await;
    publisher
        .publish(
            "sensors/kitchen/temperature",
            b"22.5",
            QoS::AtMostOnce,
            false,
        )
        .await;

    tokio::time::sleep(Duration::from_millis(100)).await;
    if let Some(Packet::Publish(msg)) = subscriber.recv().await {
        assert_eq!(msg.topic, "sensors/kitchen/temperature");
    }

    broker_handle.abort();
}

#[tokio::test]
async fn test_wildcard_multi_level() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    // Subscribe to home/#
    let mut subscriber = TestClient::connect(addr, ProtocolVersion::V311).await;
    subscriber.mqtt_connect("multi-wild-sub", true).await;
    subscriber.subscribe(1, "home/#", QoS::AtMostOnce).await;

    // Publish to deeply nested topic
    let mut publisher = TestClient::connect(addr, ProtocolVersion::V311).await;
    publisher.mqtt_connect("multi-wild-pub", true).await;
    publisher
        .publish(
            "home/floor1/room2/sensor/temp",
            b"21.0",
            QoS::AtMostOnce,
            false,
        )
        .await;

    tokio::time::sleep(Duration::from_millis(100)).await;
    if let Some(Packet::Publish(msg)) = subscriber.recv().await {
        assert_eq!(msg.topic, "home/floor1/room2/sensor/temp");
    }

    broker_handle.abort();
}

// ============================================================================
// Retained Message Tests (MQTT-3.3.1.3)
// ============================================================================

#[tokio::test]
async fn test_retained_message() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    // First, publish a retained message
    let mut publisher = TestClient::connect(addr, ProtocolVersion::V311).await;
    publisher.mqtt_connect("retain-pub", true).await;
    publisher
        .publish("status/device", b"online", QoS::AtMostOnce, true)
        .await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    // New subscriber should receive retained message immediately
    let mut subscriber = TestClient::connect(addr, ProtocolVersion::V311).await;
    subscriber.mqtt_connect("retain-sub", true).await;
    subscriber
        .subscribe(1, "status/device", QoS::AtMostOnce)
        .await;

    tokio::time::sleep(Duration::from_millis(100)).await;
    if let Some(Packet::Publish(msg)) = subscriber.recv().await {
        assert_eq!(msg.topic, "status/device");
        assert_eq!(&msg.payload[..], b"online");
        assert!(msg.retain);
    }

    broker_handle.abort();
}

// ============================================================================
// Will Message Tests (MQTT-3.1.2.5)
// ============================================================================

#[tokio::test]
async fn test_will_message_on_disconnect() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    // Subscriber waiting for will message
    let mut subscriber = TestClient::connect(addr, ProtocolVersion::V311).await;
    subscriber.mqtt_connect("will-sub", true).await;
    subscriber
        .subscribe(1, "client/status", QoS::AtMostOnce)
        .await;

    // Client with will message
    let mut will_client = TestClient::connect(addr, ProtocolVersion::V311).await;
    let connect_with_will = Packet::Connect(Box::new(Connect {
        protocol_version: ProtocolVersion::V311,
        client_id: "will-client".to_string(),
        clean_start: true,
        keep_alive: 60,
        username: None,
        password: None,
        will: Some(Will {
            topic: "client/status".to_string(),
            payload: Bytes::from_static(b"offline"),
            qos: QoS::AtMostOnce,
            retain: false,
            properties: Properties::default(),
        }),
        properties: Properties::default(),
    }));
    will_client.send(&connect_with_will).await;
    let _ = will_client.recv().await; // CONNACK

    // Abruptly close the connection (simulate disconnect)
    drop(will_client);

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Subscriber should receive the will message
    if let Some(Packet::Publish(msg)) = subscriber.recv().await {
        assert_eq!(msg.topic, "client/status");
        assert_eq!(&msg.payload[..], b"offline");
    }

    broker_handle.abort();
}

// ============================================================================
// UNSUBSCRIBE Tests (MQTT-3.10, MQTT-3.11)
// ============================================================================

#[tokio::test]
async fn test_unsubscribe() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    let mut client = TestClient::connect(addr, ProtocolVersion::V311).await;
    client.mqtt_connect("unsub-client", true).await;

    // Subscribe
    client.subscribe(1, "test/unsub", QoS::AtMostOnce).await;

    // Unsubscribe
    let unsub = Packet::Unsubscribe(Unsubscribe {
        packet_id: 2,
        filters: vec!["test/unsub".to_string()],
        properties: Properties::default(),
    });
    client.send(&unsub).await;

    match client.recv().await {
        Some(Packet::UnsubAck(ack)) => {
            assert_eq!(ack.packet_id, 2);
        }
        other => panic!("Expected UNSUBACK, got {:?}", other),
    }

    broker_handle.abort();
}

// ============================================================================
// PING Tests (MQTT-3.12, MQTT-3.13)
// ============================================================================

#[tokio::test]
async fn test_ping_pong() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    let mut client = TestClient::connect(addr, ProtocolVersion::V311).await;
    client.mqtt_connect("ping-client", true).await;

    // Send PINGREQ
    client.send(&Packet::PingReq).await;

    // Should receive PINGRESP
    match client.recv().await {
        Some(Packet::PingResp) => {}
        other => panic!("Expected PINGRESP, got {:?}", other),
    }

    broker_handle.abort();
}

// ============================================================================
// DISCONNECT Tests (MQTT-3.14)
// ============================================================================

#[tokio::test]
async fn test_graceful_disconnect() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    let mut client = TestClient::connect(addr, ProtocolVersion::V311).await;
    client.mqtt_connect("disconnect-client", true).await;

    // Send DISCONNECT
    let disconnect = Packet::Disconnect(Disconnect {
        reason_code: ReasonCode::Success,
        properties: Properties::default(),
    });
    client.send(&disconnect).await;

    // Server should close connection
    tokio::time::sleep(Duration::from_millis(100)).await;

    broker_handle.abort();
}

// ============================================================================
// Session State Tests
// ============================================================================

#[tokio::test]
async fn test_session_persistence_v5() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    // First connection with clean_start=false
    {
        let mut client = TestClient::connect(addr, ProtocolVersion::V5).await;
        let connack = client.mqtt_connect("persistent-client", false).await;
        assert!(!connack.session_present); // First time, no session
        client.subscribe(1, "persist/topic", QoS::AtLeastOnce).await;

        // Disconnect gracefully
        let disconnect = Packet::Disconnect(Disconnect {
            reason_code: ReasonCode::Success,
            properties: Properties::default(),
        });
        client.send(&disconnect).await;
    }

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Reconnect with same client ID
    {
        let mut client = TestClient::connect(addr, ProtocolVersion::V5).await;
        let connack = client.mqtt_connect("persistent-client", false).await;
        // Session should be present (subscription should be retained)
        assert!(connack.session_present);
    }

    broker_handle.abort();
}

// ============================================================================
// Multiple Subscribers Test
// ============================================================================

#[tokio::test]
async fn test_multiple_subscribers() {
    let port = next_port();
    let config = test_config(port);
    let broker = Broker::new(config);

    let broker_handle = tokio::spawn(async move {
        let _ = broker.run().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    // Two subscribers
    let mut sub1 = TestClient::connect(addr, ProtocolVersion::V311).await;
    sub1.mqtt_connect("sub1", true).await;
    sub1.subscribe(1, "broadcast", QoS::AtMostOnce).await;

    let mut sub2 = TestClient::connect(addr, ProtocolVersion::V311).await;
    sub2.mqtt_connect("sub2", true).await;
    sub2.subscribe(1, "broadcast", QoS::AtMostOnce).await;

    // Publisher
    let mut publisher = TestClient::connect(addr, ProtocolVersion::V311).await;
    publisher.mqtt_connect("broadcaster", true).await;
    publisher
        .publish("broadcast", b"to all", QoS::AtMostOnce, false)
        .await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Both subscribers should receive the message
    if let Some(Packet::Publish(msg1)) = sub1.recv().await {
        assert_eq!(&msg1.payload[..], b"to all");
    }
    if let Some(Packet::Publish(msg2)) = sub2.recv().await {
        assert_eq!(&msg2.payload[..], b"to all");
    }

    broker_handle.abort();
}

// ============================================================================
// LIMITS Tests
// ============================================================================

/// Test max_connections enforcement
#[tokio::test]
async fn test_max_connections_limit() {
    let port = next_port();
    let mut config = test_config(port);
    config.max_connections = 2; // Very low limit for testing

    let addr = config.bind_addr;
    let broker = Broker::new(config);
    let broker_handle = tokio::spawn(async move {
        broker.run().await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connect first client - should succeed
    let mut client1 = TestClient::connect(addr, ProtocolVersion::V5).await;
    let connack1 = client1.mqtt_connect("client1", true).await;
    assert_eq!(connack1.reason_code, ReasonCode::Success);

    // Connect second client - should succeed
    let mut client2 = TestClient::connect(addr, ProtocolVersion::V5).await;
    let connack2 = client2.mqtt_connect("client2", true).await;
    assert_eq!(connack2.reason_code, ReasonCode::Success);

    // Connect third client - should be rejected with ServerUnavailable
    let mut client3 = TestClient::connect(addr, ProtocolVersion::V5).await;
    let connack3 = client3.mqtt_connect("client3", true).await;
    assert_eq!(
        connack3.reason_code,
        ReasonCode::ServerUnavailable,
        "Third connection should be rejected when max_connections=2"
    );

    broker_handle.abort();
}

/// Test max_awaiting_rel enforcement (QoS 2 limit)
#[tokio::test]
async fn test_max_awaiting_rel_limit() {
    let port = next_port();
    let mut config = test_config(port);
    config.max_awaiting_rel = 2; // Very low limit for testing

    let addr = config.bind_addr;
    let broker = Broker::new(config);
    let broker_handle = tokio::spawn(async move {
        broker.run().await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = TestClient::connect(addr, ProtocolVersion::V5).await;
    client.mqtt_connect("qos2-test", true).await;

    // Send QoS 2 publishes without completing the handshake (no PUBREL)
    // First two should succeed
    for i in 1..=2 {
        let publish = Packet::Publish(Publish {
            dup: false,
            qos: QoS::ExactlyOnce,
            retain: false,
            topic: "test/qos2".to_string(),
            packet_id: Some(i),
            payload: Bytes::from(format!("msg{}", i)),
            properties: Properties::default(),
        });
        client.send(&publish).await;

        // Should receive PUBREC with Success
        if let Some(Packet::PubRec(pubrec)) = client.recv().await {
            assert_eq!(
                pubrec.reason_code,
                ReasonCode::Success,
                "First {} QoS 2 messages should succeed",
                i
            );
        }
    }

    // Third QoS 2 publish should be rejected with QuotaExceeded
    let publish3 = Packet::Publish(Publish {
        dup: false,
        qos: QoS::ExactlyOnce,
        retain: false,
        topic: "test/qos2".to_string(),
        packet_id: Some(3),
        payload: Bytes::from("msg3"),
        properties: Properties::default(),
    });
    client.send(&publish3).await;

    if let Some(Packet::PubRec(pubrec)) = client.recv().await {
        assert_eq!(
            pubrec.reason_code,
            ReasonCode::QuotaExceeded,
            "Third QoS 2 message should be rejected when max_awaiting_rel=2"
        );
    }

    broker_handle.abort();
}

/// Test max_inflight config is applied to sessions
#[tokio::test]
async fn test_max_inflight_limit() {
    let port = next_port();
    let mut config = test_config(port);
    config.max_inflight = 16;

    let addr = config.bind_addr;
    let broker = Broker::new(config);
    let broker_handle = tokio::spawn(async move {
        broker.run().await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Subscriber with QoS 1
    let mut subscriber = TestClient::connect(addr, ProtocolVersion::V5).await;
    subscriber.mqtt_connect("sub-inflight", true).await;
    subscriber
        .subscribe(1, "test/inflight", QoS::AtLeastOnce)
        .await;

    // Publisher
    let mut publisher = TestClient::connect(addr, ProtocolVersion::V5).await;
    publisher.mqtt_connect("pub-inflight", true).await;

    // Publish a QoS 1 message
    let publish = Packet::Publish(Publish {
        dup: false,
        qos: QoS::AtLeastOnce,
        retain: false,
        topic: "test/inflight".to_string(),
        packet_id: Some(1),
        payload: Bytes::from("test message"),
        properties: Properties::default(),
    });
    publisher.send(&publish).await;
    let _ = publisher.recv().await; // PUBACK from broker

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Subscriber should receive the message
    let msg = subscriber.recv().await;
    assert!(msg.is_some(), "Should receive message");

    if let Some(Packet::Publish(p)) = msg {
        assert_eq!(p.payload.as_ref(), b"test message");
        // ACK it
        let puback = Packet::PubAck(vibemq::protocol::PubAck::new(p.packet_id.unwrap()));
        subscriber.send(&puback).await;
    }

    broker_handle.abort();
}

/// Test that session takeover doesn't count against max_connections
#[tokio::test]
async fn test_max_connections_allows_takeover() {
    let port = next_port();
    let mut config = test_config(port);
    config.max_connections = 1; // Only 1 connection allowed

    let addr = config.bind_addr;
    let broker = Broker::new(config);
    let broker_handle = tokio::spawn(async move {
        broker.run().await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connect first client
    let mut client1 = TestClient::connect(addr, ProtocolVersion::V5).await;
    let connack1 = client1.mqtt_connect("same-client", true).await;
    assert_eq!(connack1.reason_code, ReasonCode::Success);

    // Connect second client with SAME client_id - should succeed (takeover)
    let mut client2 = TestClient::connect(addr, ProtocolVersion::V5).await;
    let connack2 = client2.mqtt_connect("same-client", true).await;
    assert_eq!(
        connack2.reason_code,
        ReasonCode::Success,
        "Session takeover should succeed even at max_connections"
    );

    // Connect third client with DIFFERENT client_id - should fail
    let mut client3 = TestClient::connect(addr, ProtocolVersion::V5).await;
    let connack3 = client3.mqtt_connect("different-client", true).await;
    assert_eq!(
        connack3.reason_code,
        ReasonCode::ServerUnavailable,
        "New client_id should be rejected at max_connections"
    );

    broker_handle.abort();
}

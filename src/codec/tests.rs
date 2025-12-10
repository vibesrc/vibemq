//! Comprehensive MQTT Codec Tests
//!
//! Tests for encoding and decoding all MQTT packet types for both v3.1.1 and v5.0
//! Based on MQTT specification sections 2 and 3.

#![allow(clippy::field_reassign_with_default)]

use bytes::{Bytes, BytesMut};
use pretty_assertions::assert_eq;

use crate::codec::{Decoder, Encoder};
use crate::protocol::{
    Auth, ConnAck, Connect, DecodeError, Disconnect, Packet, Properties, ProtocolVersion, PubAck,
    PubComp, PubRec, PubRel, Publish, QoS, ReasonCode, RetainHandling, SubAck, Subscribe,
    Subscription, SubscriptionOptions, UnsubAck, Unsubscribe, Will,
};

// ============================================================================
// Helper functions for building test packets
// ============================================================================

fn encode_packet(packet: &Packet, version: ProtocolVersion) -> BytesMut {
    let encoder = Encoder::new(version);
    let mut buf = BytesMut::new();
    encoder.encode(packet, &mut buf).unwrap();
    buf
}

fn decode_packet(buf: &[u8], version: Option<ProtocolVersion>) -> Result<Packet, DecodeError> {
    let mut decoder = Decoder::new();
    if let Some(v) = version {
        decoder.set_protocol_version(v);
    }
    match decoder.decode(buf)? {
        Some((packet, _)) => Ok(packet),
        None => Err(DecodeError::InsufficientData),
    }
}

// ============================================================================
// CONNECT Packet Tests (MQTT-3.1)
// ============================================================================

#[test]
fn test_connect_v311_minimal() {
    // Minimal v3.1.1 CONNECT: clean session, empty client ID
    let packet = Packet::Connect(Box::new(Connect {
        protocol_version: ProtocolVersion::V311,
        client_id: String::new(),
        clean_start: true,
        keep_alive: 60,
        username: None,
        password: None,
        will: None,
        properties: Properties::default(),
    }));

    let encoded = encode_packet(&packet, ProtocolVersion::V311);
    let decoded = decode_packet(&encoded, None).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_connect_v311_full() {
    // Full v3.1.1 CONNECT with all fields
    let packet = Packet::Connect(Box::new(Connect {
        protocol_version: ProtocolVersion::V311,
        client_id: "test-client-123".to_string(),
        clean_start: false,
        keep_alive: 300,
        username: Some("user".to_string()),
        password: Some(Bytes::from("password")),
        will: Some(Will {
            topic: "last/will/topic".to_string(),
            payload: Bytes::from("goodbye"),
            qos: QoS::AtLeastOnce,
            retain: true,
            properties: Properties::default(),
        }),
        properties: Properties::default(),
    }));

    let encoded = encode_packet(&packet, ProtocolVersion::V311);
    let decoded = decode_packet(&encoded, None).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_connect_v5_minimal() {
    // Minimal v5.0 CONNECT
    let packet = Packet::Connect(Box::new(Connect {
        protocol_version: ProtocolVersion::V5,
        client_id: String::new(),
        clean_start: true,
        keep_alive: 60,
        username: None,
        password: None,
        will: None,
        properties: Properties::default(),
    }));

    let encoded = encode_packet(&packet, ProtocolVersion::V5);
    let decoded = decode_packet(&encoded, None).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_connect_v5_with_properties() {
    // v5.0 CONNECT with properties
    let mut props = Properties::default();
    props.session_expiry_interval = Some(3600);
    props.receive_maximum = Some(100);
    props.maximum_packet_size = Some(1024 * 1024);
    props.topic_alias_maximum = Some(10);
    props.request_response_information = Some(1);
    props.request_problem_information = Some(1);

    let packet = Packet::Connect(Box::new(Connect {
        protocol_version: ProtocolVersion::V5,
        client_id: "client-v5".to_string(),
        clean_start: true,
        keep_alive: 120,
        username: None,
        password: None,
        will: None,
        properties: props,
    }));

    let encoded = encode_packet(&packet, ProtocolVersion::V5);
    let decoded = decode_packet(&encoded, None).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_connect_v5_with_will_properties() {
    // v5.0 CONNECT with will message and will properties
    let mut will_props = Properties::default();
    will_props.will_delay_interval = Some(30);
    will_props.message_expiry_interval = Some(3600);
    will_props.content_type = Some("application/json".to_string());
    will_props.payload_format_indicator = Some(1);

    let packet = Packet::Connect(Box::new(Connect {
        protocol_version: ProtocolVersion::V5,
        client_id: "will-test".to_string(),
        clean_start: true,
        keep_alive: 60,
        username: None,
        password: None,
        will: Some(Will {
            topic: "status/offline".to_string(),
            payload: Bytes::from(r#"{"status":"offline"}"#),
            qos: QoS::ExactlyOnce,
            retain: false,
            properties: will_props,
        }),
        properties: Properties::default(),
    }));

    let encoded = encode_packet(&packet, ProtocolVersion::V5);
    let decoded = decode_packet(&encoded, None).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_connect_invalid_protocol_name() {
    // Invalid protocol name should fail
    let invalid = [
        0x10, 0x0C, // CONNECT, remaining length
        0x00, 0x04, b'X', b'Q', b'T', b'T', // Invalid "XQTT"
        0x04, // Protocol level 4
        0x02, // Clean session
        0x00, 0x3C, // Keep alive 60
        0x00, 0x00, // Empty client ID
    ];
    let result = decode_packet(&invalid, None);
    assert!(matches!(result, Err(DecodeError::InvalidProtocolName)));
}

#[test]
fn test_connect_invalid_protocol_version() {
    // Invalid protocol version should fail
    let invalid = [
        0x10, 0x0C, // CONNECT, remaining length
        0x00, 0x04, b'M', b'Q', b'T', b'T', // "MQTT"
        0x06, // Invalid protocol level 6
        0x02, // Clean session
        0x00, 0x3C, // Keep alive 60
        0x00, 0x00, // Empty client ID
    ];
    let result = decode_packet(&invalid, None);
    assert!(matches!(
        result,
        Err(DecodeError::InvalidProtocolVersion(6))
    ));
}

#[test]
fn test_connect_reserved_bit_set() {
    // Reserved bit in connect flags must be 0
    let invalid = [
        0x10, 0x0C, // CONNECT, remaining length
        0x00, 0x04, b'M', b'Q', b'T', b'T', // "MQTT"
        0x04, // Protocol level 4
        0x03, // Clean session + reserved bit set (invalid)
        0x00, 0x3C, // Keep alive 60
        0x00, 0x00, // Empty client ID
    ];
    let result = decode_packet(&invalid, None);
    assert!(matches!(result, Err(DecodeError::InvalidFlags)));
}

#[test]
fn test_connect_will_qos_invalid() {
    // Will QoS 3 is invalid
    let invalid = [
        0x10, 0x0C, // CONNECT, remaining length
        0x00, 0x04, b'M', b'Q', b'T', b'T', // "MQTT"
        0x04, // Protocol level 4
        0x1E, // Will flag + QoS 3 (invalid)
        0x00, 0x3C, // Keep alive 60
        0x00, 0x00, // Empty client ID
    ];
    let result = decode_packet(&invalid, None);
    assert!(matches!(result, Err(DecodeError::InvalidQoS(3))));
}

#[test]
fn test_connect_will_flags_inconsistent() {
    // Will retain set but will flag not set - invalid
    let invalid = [
        0x10, 0x0C, // CONNECT, remaining length
        0x00, 0x04, b'M', b'Q', b'T', b'T', // "MQTT"
        0x04, // Protocol level 4
        0x22, // Clean session + will retain (but no will flag)
        0x00, 0x3C, // Keep alive 60
        0x00, 0x00, // Empty client ID
    ];
    let result = decode_packet(&invalid, None);
    assert!(matches!(result, Err(DecodeError::InvalidFlags)));
}

// ============================================================================
// CONNACK Packet Tests (MQTT-3.2)
// ============================================================================

#[test]
fn test_connack_v311_success() {
    let packet = Packet::ConnAck(ConnAck {
        session_present: false,
        reason_code: ReasonCode::Success,
        properties: Properties::default(),
    });

    let encoded = encode_packet(&packet, ProtocolVersion::V311);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V311)).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_connack_v311_session_present() {
    let packet = Packet::ConnAck(ConnAck {
        session_present: true,
        reason_code: ReasonCode::Success,
        properties: Properties::default(),
    });

    let encoded = encode_packet(&packet, ProtocolVersion::V311);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V311)).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_connack_v5_with_properties() {
    let mut props = Properties::default();
    props.session_expiry_interval = Some(3600);
    props.receive_maximum = Some(50);
    props.maximum_qos = Some(2);
    props.retain_available = Some(1);
    props.maximum_packet_size = Some(1048576);
    props.assigned_client_identifier = Some("generated-id".to_string());
    props.topic_alias_maximum = Some(100);
    props.wildcard_subscription_available = Some(1);
    props.subscription_identifier_available = Some(1);
    props.shared_subscription_available = Some(1);
    props.server_keep_alive = Some(120);

    let packet = Packet::ConnAck(ConnAck {
        session_present: false,
        reason_code: ReasonCode::Success,
        properties: props,
    });

    let encoded = encode_packet(&packet, ProtocolVersion::V5);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V5)).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_connack_all_v311_reason_codes() {
    // Test all v3.1.1 return codes
    let codes = [
        (0x00, ReasonCode::Success),
        (0x01, ReasonCode::UnsupportedProtocolVersion),
        (0x02, ReasonCode::ClientIdNotValid),
        (0x03, ReasonCode::ServerUnavailable),
        (0x04, ReasonCode::BadUserNameOrPassword),
        (0x05, ReasonCode::NotAuthorized),
    ];

    for (byte, expected_code) in codes {
        let data = [0x20, 0x02, 0x00, byte];
        let mut decoder = Decoder::new();
        decoder.set_protocol_version(ProtocolVersion::V311);
        let result = decoder.decode(&data).unwrap().unwrap();
        if let Packet::ConnAck(connack) = result.0 {
            assert_eq!(
                connack.reason_code, expected_code,
                "Failed for byte {:#04x}",
                byte
            );
        } else {
            panic!("Expected ConnAck packet");
        }
    }
}

// ============================================================================
// PUBLISH Packet Tests (MQTT-3.3)
// ============================================================================

#[test]
fn test_publish_qos0() {
    let packet = Packet::Publish(Publish {
        dup: false,
        qos: QoS::AtMostOnce,
        retain: false,
        topic: "test/topic".to_string(),
        packet_id: None,
        payload: Bytes::from("hello world"),
        properties: Properties::default(),
    });

    let encoded = encode_packet(&packet, ProtocolVersion::V311);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V311)).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_publish_qos1() {
    let packet = Packet::Publish(Publish {
        dup: false,
        qos: QoS::AtLeastOnce,
        retain: false,
        topic: "test/topic".to_string(),
        packet_id: Some(1234),
        payload: Bytes::from("hello world"),
        properties: Properties::default(),
    });

    let encoded = encode_packet(&packet, ProtocolVersion::V311);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V311)).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_publish_qos2() {
    let packet = Packet::Publish(Publish {
        dup: true,
        qos: QoS::ExactlyOnce,
        retain: true,
        topic: "sensors/temp".to_string(),
        packet_id: Some(65535),
        payload: Bytes::from(r#"{"temp": 25.5}"#),
        properties: Properties::default(),
    });

    let encoded = encode_packet(&packet, ProtocolVersion::V311);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V311)).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_publish_v5_with_properties() {
    let mut props = Properties::default();
    props.payload_format_indicator = Some(1);
    props.message_expiry_interval = Some(3600);
    props.topic_alias = Some(5);
    props.response_topic = Some("reply/to".to_string());
    props.correlation_data = Some(Bytes::from("correlation-123"));
    props.content_type = Some("application/json".to_string());

    let packet = Packet::Publish(Publish {
        dup: false,
        qos: QoS::AtLeastOnce,
        retain: false,
        topic: "data/stream".to_string(),
        packet_id: Some(100),
        payload: Bytes::from(r#"{"value": 42}"#),
        properties: props,
    });

    let encoded = encode_packet(&packet, ProtocolVersion::V5);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V5)).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_publish_empty_payload() {
    let packet = Packet::Publish(Publish {
        dup: false,
        qos: QoS::AtMostOnce,
        retain: true,
        topic: "clear/retained".to_string(),
        packet_id: None,
        payload: Bytes::new(), // Empty payload clears retained message
        properties: Properties::default(),
    });

    let encoded = encode_packet(&packet, ProtocolVersion::V311);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V311)).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_publish_dup_must_be_zero_for_qos0() {
    // Per MQTT-3.3.1-2: DUP must be 0 for QoS 0
    let invalid = [
        0x38, 0x0D, // PUBLISH with DUP=1, QoS=0 (invalid)
        0x00, 0x05, b't', b'o', b'p', b'i', b'c', // topic
        b'p', b'a', b'y', b'l', b'o', b'a', b'd', // payload
    ];
    let result = decode_packet(&invalid, Some(ProtocolVersion::V311));
    assert!(matches!(
        result,
        Err(DecodeError::MalformedPacket("DUP must be 0 for QoS 0"))
    ));
}

#[test]
fn test_publish_topic_with_wildcard_invalid() {
    // Topic name cannot contain wildcards
    let invalid = [
        0x30, 0x08, // PUBLISH QoS 0
        0x00, 0x04, b't', b'e', b's', b'+', // topic with + (invalid)
        b'h', b'i', // payload
    ];
    let result = decode_packet(&invalid, Some(ProtocolVersion::V311));
    assert!(matches!(
        result,
        Err(DecodeError::MalformedPacket("topic contains wildcard"))
    ));
}

#[test]
fn test_publish_packet_id_zero_invalid() {
    // Packet ID 0 is invalid for QoS > 0
    let invalid = [
        0x32, 0x09, // PUBLISH QoS 1
        0x00, 0x04, b't', b'e', b's', b't', // topic
        0x00, 0x00, // packet ID = 0 (invalid)
        b'x', // payload
    ];
    let result = decode_packet(&invalid, Some(ProtocolVersion::V311));
    assert!(matches!(
        result,
        Err(DecodeError::MalformedPacket("packet id cannot be 0"))
    ));
}

// ============================================================================
// PUBACK/PUBREC/PUBREL/PUBCOMP Tests (MQTT-3.4 to 3.7)
// ============================================================================

#[test]
fn test_puback_v311() {
    let packet = Packet::PubAck(PubAck::new(1234));
    let encoded = encode_packet(&packet, ProtocolVersion::V311);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V311)).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_puback_v5_with_reason() {
    let packet = Packet::PubAck(PubAck {
        packet_id: 5678,
        reason_code: ReasonCode::NoMatchingSubscribers,
        properties: Properties::default(),
    });
    let encoded = encode_packet(&packet, ProtocolVersion::V5);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V5)).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_pubrec_v5() {
    let packet = Packet::PubRec(PubRec {
        packet_id: 9999,
        reason_code: ReasonCode::Success,
        properties: Properties::default(),
    });
    let encoded = encode_packet(&packet, ProtocolVersion::V5);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V5)).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_pubrel_flags() {
    // PUBREL must have flags 0010
    let packet = Packet::PubRel(PubRel::new(1000));
    let encoded = encode_packet(&packet, ProtocolVersion::V311);
    // Verify first byte has correct flags (0x62 = packet type 6 + flags 0010)
    assert_eq!(encoded[0], 0x62);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V311)).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_pubrel_invalid_flags() {
    // PUBREL with wrong flags should fail
    let invalid = [0x60, 0x02, 0x00, 0x01]; // flags = 0000 instead of 0010
    let result = decode_packet(&invalid, Some(ProtocolVersion::V311));
    assert!(matches!(result, Err(DecodeError::InvalidFlags)));
}

#[test]
fn test_pubcomp_v5_with_reason() {
    let packet = Packet::PubComp(PubComp {
        packet_id: 7777,
        reason_code: ReasonCode::PacketIdNotFound,
        properties: Properties::default(),
    });
    let encoded = encode_packet(&packet, ProtocolVersion::V5);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V5)).unwrap();
    assert_eq!(packet, decoded);
}

// ============================================================================
// SUBSCRIBE Packet Tests (MQTT-3.8)
// ============================================================================

#[test]
fn test_subscribe_single_topic() {
    let packet = Packet::Subscribe(Subscribe {
        packet_id: 1,
        subscriptions: vec![Subscription {
            filter: "test/topic".to_string(),
            options: SubscriptionOptions::default(),
        }],
        properties: Properties::default(),
    });

    let encoded = encode_packet(&packet, ProtocolVersion::V311);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V311)).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_subscribe_multiple_topics() {
    let packet = Packet::Subscribe(Subscribe {
        packet_id: 100,
        subscriptions: vec![
            Subscription {
                filter: "sensors/+/temperature".to_string(),
                options: SubscriptionOptions {
                    qos: QoS::AtLeastOnce,
                    ..Default::default()
                },
            },
            Subscription {
                filter: "alerts/#".to_string(),
                options: SubscriptionOptions {
                    qos: QoS::ExactlyOnce,
                    ..Default::default()
                },
            },
            Subscription {
                filter: "status".to_string(),
                options: SubscriptionOptions {
                    qos: QoS::AtMostOnce,
                    ..Default::default()
                },
            },
        ],
        properties: Properties::default(),
    });

    let encoded = encode_packet(&packet, ProtocolVersion::V311);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V311)).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_subscribe_v5_with_options() {
    let mut props = Properties::default();
    props.subscription_identifiers = vec![42];

    let packet = Packet::Subscribe(Subscribe {
        packet_id: 200,
        subscriptions: vec![Subscription {
            filter: "data/+/events".to_string(),
            options: SubscriptionOptions {
                qos: QoS::ExactlyOnce,
                no_local: true,
                retain_as_published: true,
                retain_handling: RetainHandling::DoNotSend,
            },
        }],
        properties: props,
    });

    let encoded = encode_packet(&packet, ProtocolVersion::V5);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V5)).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_subscribe_flags() {
    // SUBSCRIBE must have flags 0010
    let packet = Packet::Subscribe(Subscribe {
        packet_id: 1,
        subscriptions: vec![Subscription {
            filter: "test".to_string(),
            options: SubscriptionOptions::default(),
        }],
        properties: Properties::default(),
    });
    let encoded = encode_packet(&packet, ProtocolVersion::V311);
    // Verify first byte has correct flags (0x82 = packet type 8 + flags 0010)
    assert_eq!(encoded[0], 0x82);
}

#[test]
fn test_subscribe_invalid_flags() {
    // SUBSCRIBE with wrong flags should fail
    let invalid = [
        0x80, 0x08, // SUBSCRIBE with flags = 0000 (invalid)
        0x00, 0x01, // packet ID
        0x00, 0x03, b't', b'o', b'p', // topic
        0x00, // QoS
    ];
    let result = decode_packet(&invalid, Some(ProtocolVersion::V311));
    assert!(matches!(result, Err(DecodeError::InvalidFlags)));
}

#[test]
fn test_subscribe_empty_topics_invalid() {
    // SUBSCRIBE must have at least one topic
    let invalid = [
        0x82, 0x02, // SUBSCRIBE
        0x00, 0x01, // packet ID only, no topics
    ];
    let result = decode_packet(&invalid, Some(ProtocolVersion::V311));
    assert!(matches!(
        result,
        Err(DecodeError::MalformedPacket(
            "SUBSCRIBE must have at least one topic"
        ))
    ));
}

#[test]
fn test_subscribe_packet_id_zero_invalid() {
    // Packet ID 0 is invalid
    let invalid = [
        0x82, 0x07, // SUBSCRIBE
        0x00, 0x00, // packet ID = 0 (invalid)
        0x00, 0x02, b't', b'p', // topic
        0x00, // QoS
    ];
    let result = decode_packet(&invalid, Some(ProtocolVersion::V311));
    assert!(matches!(
        result,
        Err(DecodeError::MalformedPacket("packet id cannot be 0"))
    ));
}

#[test]
fn test_subscribe_v311_ignores_upper_bits() {
    // In MQTT v3.1.1, only bits 0-1 (QoS) are used; bits 2-7 are reserved
    // The decoder should only extract QoS and default v5.0 options
    // Manually construct a SUBSCRIBE packet with upper bits set
    // Remaining length: packet_id(2) + topic_len(2) + topic(4) + options(1) = 9
    let raw = [
        0x82, 0x09, // SUBSCRIBE with correct flags, remaining length = 9
        0x00, 0x01, // packet ID = 1
        0x00, 0x04, b't', b'e', b's', b't', // topic "test"
        0x3E, // QoS 2 (0x02) + upper bits set (0x3C) - would be invalid in v5
    ];

    let result = decode_packet(&raw, Some(ProtocolVersion::V311)).unwrap();
    if let Packet::Subscribe(sub) = result {
        assert_eq!(sub.subscriptions.len(), 1);
        // QoS should be extracted correctly
        assert_eq!(sub.subscriptions[0].options.qos, QoS::ExactlyOnce);
        // v5.0-only options should be defaults (not parsed from upper bits)
        assert!(!sub.subscriptions[0].options.no_local);
        assert!(!sub.subscriptions[0].options.retain_as_published);
        assert_eq!(
            sub.subscriptions[0].options.retain_handling,
            RetainHandling::SendAtSubscribe
        );
    } else {
        panic!("Expected Subscribe packet");
    }
}

#[test]
fn test_subscribe_v5_parses_all_options() {
    // In MQTT v5.0, all option bits are parsed
    // Remaining length: packet_id(2) + props_len(1) + topic_len(2) + topic(4) + options(1) = 10
    let raw = [
        0x82, 0x0A, // SUBSCRIBE with correct flags, remaining length = 10
        0x00, 0x01, // packet ID = 1
        0x00, // properties length = 0
        0x00, 0x04, b't', b'e', b's', b't', // topic "test"
        0x2E, // QoS 2 + no_local (0x04) + retain_as_published (0x08) + retain_handling=2 (0x20)
    ];

    let result = decode_packet(&raw, Some(ProtocolVersion::V5)).unwrap();
    if let Packet::Subscribe(sub) = result {
        assert_eq!(sub.subscriptions.len(), 1);
        assert_eq!(sub.subscriptions[0].options.qos, QoS::ExactlyOnce);
        assert!(sub.subscriptions[0].options.no_local);
        assert!(sub.subscriptions[0].options.retain_as_published);
        assert_eq!(
            sub.subscriptions[0].options.retain_handling,
            RetainHandling::DoNotSend
        );
    } else {
        panic!("Expected Subscribe packet");
    }
}

#[test]
fn test_subscribe_v5_rejects_reserved_bits() {
    // In MQTT v5.0, reserved bits 6-7 must be zero
    // Remaining length: packet_id(2) + props_len(1) + topic_len(2) + topic(4) + options(1) = 10
    let raw = [
        0x82, 0x0A, // SUBSCRIBE with correct flags, remaining length = 10
        0x00, 0x01, // packet ID = 1
        0x00, // properties length = 0
        0x00, 0x04, b't', b'e', b's', b't', // topic "test"
        0xC2, // QoS 2 + reserved bits 6-7 set (invalid)
    ];

    let result = decode_packet(&raw, Some(ProtocolVersion::V5));
    assert!(matches!(
        result,
        Err(DecodeError::InvalidSubscriptionOptions)
    ));
}

// ============================================================================
// SUBACK Packet Tests (MQTT-3.9)
// ============================================================================

#[test]
fn test_suback_v311() {
    let packet = Packet::SubAck(SubAck {
        packet_id: 1,
        reason_codes: vec![
            ReasonCode::GrantedQoS1,
            ReasonCode::GrantedQoS2,
            ReasonCode::Success,
        ],
        properties: Properties::default(),
    });

    let encoded = encode_packet(&packet, ProtocolVersion::V311);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V311)).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_suback_v5_with_properties() {
    let mut props = Properties::default();
    props.reason_string = Some("Subscription accepted".to_string());

    let packet = Packet::SubAck(SubAck {
        packet_id: 500,
        reason_codes: vec![ReasonCode::GrantedQoS2],
        properties: props,
    });

    let encoded = encode_packet(&packet, ProtocolVersion::V5);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V5)).unwrap();
    assert_eq!(packet, decoded);
}

// ============================================================================
// UNSUBSCRIBE Packet Tests (MQTT-3.10)
// ============================================================================

#[test]
fn test_unsubscribe_single_topic() {
    let packet = Packet::Unsubscribe(Unsubscribe {
        packet_id: 1,
        filters: vec!["test/topic".to_string()],
        properties: Properties::default(),
    });

    let encoded = encode_packet(&packet, ProtocolVersion::V311);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V311)).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_unsubscribe_multiple_topics() {
    let packet = Packet::Unsubscribe(Unsubscribe {
        packet_id: 300,
        filters: vec![
            "sensors/+/temperature".to_string(),
            "alerts/#".to_string(),
            "status".to_string(),
        ],
        properties: Properties::default(),
    });

    let encoded = encode_packet(&packet, ProtocolVersion::V311);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V311)).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_unsubscribe_flags() {
    // UNSUBSCRIBE must have flags 0010
    let packet = Packet::Unsubscribe(Unsubscribe {
        packet_id: 1,
        filters: vec!["test".to_string()],
        properties: Properties::default(),
    });
    let encoded = encode_packet(&packet, ProtocolVersion::V311);
    // Verify first byte has correct flags (0xA2 = packet type 10 + flags 0010)
    assert_eq!(encoded[0], 0xA2);
}

#[test]
fn test_unsubscribe_empty_topics_invalid() {
    // UNSUBSCRIBE must have at least one topic
    let invalid = [
        0xA2, 0x02, // UNSUBSCRIBE
        0x00, 0x01, // packet ID only, no topics
    ];
    let result = decode_packet(&invalid, Some(ProtocolVersion::V311));
    assert!(matches!(
        result,
        Err(DecodeError::MalformedPacket(
            "UNSUBSCRIBE must have at least one topic"
        ))
    ));
}

// ============================================================================
// UNSUBACK Packet Tests (MQTT-3.11)
// ============================================================================

#[test]
fn test_unsuback_v311() {
    let packet = Packet::UnsubAck(UnsubAck {
        packet_id: 1,
        reason_codes: vec![], // v3.1.1 has no reason codes
        properties: Properties::default(),
    });

    let encoded = encode_packet(&packet, ProtocolVersion::V311);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V311)).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_unsuback_v5() {
    let packet = Packet::UnsubAck(UnsubAck {
        packet_id: 400,
        reason_codes: vec![ReasonCode::Success, ReasonCode::NoSubscriptionExisted],
        properties: Properties::default(),
    });

    let encoded = encode_packet(&packet, ProtocolVersion::V5);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V5)).unwrap();
    assert_eq!(packet, decoded);
}

// ============================================================================
// PINGREQ/PINGRESP Tests (MQTT-3.12, 3.13)
// ============================================================================

#[test]
fn test_pingreq() {
    let packet = Packet::PingReq;
    let encoded = encode_packet(&packet, ProtocolVersion::V311);
    assert_eq!(&encoded[..], &[0xC0, 0x00]);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V311)).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_pingresp() {
    let packet = Packet::PingResp;
    let encoded = encode_packet(&packet, ProtocolVersion::V311);
    assert_eq!(&encoded[..], &[0xD0, 0x00]);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V311)).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_pingreq_invalid_flags() {
    // PINGREQ must have flags 0000
    let invalid = [0xC1, 0x00]; // flags = 0001 (invalid)
    let result = decode_packet(&invalid, Some(ProtocolVersion::V311));
    assert!(matches!(result, Err(DecodeError::InvalidFlags)));
}

// ============================================================================
// DISCONNECT Packet Tests (MQTT-3.14)
// ============================================================================

#[test]
fn test_disconnect_v311() {
    let packet = Packet::Disconnect(Disconnect::default());
    let encoded = encode_packet(&packet, ProtocolVersion::V311);
    assert_eq!(&encoded[..], &[0xE0, 0x00]);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V311)).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_disconnect_v5_with_reason() {
    let packet = Packet::Disconnect(Disconnect {
        reason_code: ReasonCode::DisconnectWithWill,
        properties: Properties::default(),
    });

    let encoded = encode_packet(&packet, ProtocolVersion::V5);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V5)).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_disconnect_v5_with_properties() {
    let mut props = Properties::default();
    props.session_expiry_interval = Some(0);
    props.reason_string = Some("Closing connection".to_string());

    let packet = Packet::Disconnect(Disconnect {
        reason_code: ReasonCode::Success,
        properties: props,
    });

    let encoded = encode_packet(&packet, ProtocolVersion::V5);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V5)).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_disconnect_v311_with_payload_invalid() {
    // v3.1.1 DISCONNECT has no payload
    let invalid = [0xE0, 0x01, 0x00]; // Has 1 byte payload
    let mut decoder = Decoder::new();
    decoder.set_protocol_version(ProtocolVersion::V311);
    let result = decoder.decode(&invalid);
    assert!(matches!(
        result,
        Err(DecodeError::MalformedPacket(
            "v3.1.1 DISCONNECT has no payload"
        ))
    ));
}

// ============================================================================
// AUTH Packet Tests (MQTT-3.15, v5.0 only)
// ============================================================================

#[test]
fn test_auth_v5() {
    let mut props = Properties::default();
    props.authentication_method = Some("SCRAM-SHA-256".to_string());
    props.authentication_data = Some(Bytes::from("client-first-message"));

    let packet = Packet::Auth(Auth {
        reason_code: ReasonCode::ContinueAuthentication,
        properties: props,
    });

    let encoded = encode_packet(&packet, ProtocolVersion::V5);
    let decoded = decode_packet(&encoded, Some(ProtocolVersion::V5)).unwrap();
    assert_eq!(packet, decoded);
}

#[test]
fn test_auth_v311_invalid() {
    // AUTH is v5.0 only
    let invalid = [0xF0, 0x00]; // AUTH packet
    let mut decoder = Decoder::new();
    decoder.set_protocol_version(ProtocolVersion::V311);
    let result = decoder.decode(&invalid);
    assert!(matches!(result, Err(DecodeError::InvalidPacketType(15))));
}

// ============================================================================
// Variable Length Integer Tests
// ============================================================================

#[test]
fn test_variable_int_boundary_values() {
    use crate::codec::{read_variable_int, write_variable_int};

    let test_cases = [
        (0, vec![0x00]),
        (127, vec![0x7F]),
        (128, vec![0x80, 0x01]),
        (16383, vec![0xFF, 0x7F]),
        (16384, vec![0x80, 0x80, 0x01]),
        (2097151, vec![0xFF, 0xFF, 0x7F]),
        (2097152, vec![0x80, 0x80, 0x80, 0x01]),
        (268435455, vec![0xFF, 0xFF, 0xFF, 0x7F]),
    ];

    for (value, expected_bytes) in test_cases {
        // Test encoding
        let mut buf = BytesMut::new();
        write_variable_int(&mut buf, value).unwrap();
        assert_eq!(
            &buf[..],
            &expected_bytes[..],
            "Encoding failed for {}",
            value
        );

        // Test decoding
        let (decoded, len) = read_variable_int(&buf).unwrap();
        assert_eq!(decoded, value, "Decoding failed for {}", value);
        assert_eq!(len, expected_bytes.len());
    }
}

#[test]
fn test_variable_int_invalid() {
    use crate::codec::read_variable_int;

    // More than 4 bytes with continuation bits
    let invalid = [0x80, 0x80, 0x80, 0x80, 0x01];
    let result = read_variable_int(&invalid);
    assert!(matches!(result, Err(DecodeError::InvalidRemainingLength)));
}

// ============================================================================
// Packet Size and Bounds Tests
// ============================================================================

#[test]
fn test_packet_too_large() {
    let mut decoder = Decoder::new().with_max_packet_size(100);

    // Create a packet that claims to have a large remaining length
    let data = [
        0x30, // PUBLISH
        0xFF, 0xFF, 0xFF, 0x7F, // Remaining length = 268435455 (max)
    ];

    let result = decoder.decode(&data);
    assert!(matches!(result, Err(DecodeError::PacketTooLarge)));
}

#[test]
fn test_incomplete_packet() {
    let mut decoder = Decoder::new();

    // Incomplete CONNECT packet
    let partial = [0x10, 0x0F]; // Says 15 bytes remaining but only 2 bytes total
    let result = decoder.decode(&partial).unwrap();
    assert!(result.is_none(), "Should return None for incomplete packet");
}

#[test]
fn test_invalid_packet_type() {
    let invalid = [0x00, 0x00]; // Packet type 0 is reserved
    let result = decode_packet(&invalid, None);
    assert!(matches!(result, Err(DecodeError::InvalidPacketType(0))));
}

// ============================================================================
// UTF-8 String Tests
// ============================================================================

#[test]
fn test_string_with_null_character_invalid() {
    use crate::codec::read_string;

    // String containing null character
    let data = [0x00, 0x05, b'h', b'e', 0x00, b'l', b'o'];
    let result = read_string(&data);
    assert!(matches!(
        result,
        Err(DecodeError::MalformedPacket(
            "string contains null character"
        ))
    ));
}

#[test]
fn test_string_invalid_utf8() {
    use crate::codec::read_string;

    // Invalid UTF-8 sequence
    let data = [0x00, 0x03, 0xFF, 0xFE, 0xFD];
    let result = read_string(&data);
    assert!(matches!(result, Err(DecodeError::InvalidUtf8)));
}

// ============================================================================
// Round-trip Tests
// ============================================================================

#[test]
fn test_roundtrip_all_packet_types_v311() {
    let packets = [
        Packet::Connect(Box::new(Connect {
            protocol_version: ProtocolVersion::V311,
            client_id: "test".to_string(),
            clean_start: true,
            keep_alive: 60,
            username: None,
            password: None,
            will: None,
            properties: Properties::default(),
        })),
        Packet::Publish(Publish {
            dup: false,
            qos: QoS::AtLeastOnce,
            retain: false,
            topic: "test".to_string(),
            packet_id: Some(1),
            payload: Bytes::from("data"),
            properties: Properties::default(),
        }),
        Packet::PubAck(PubAck::new(1)),
        Packet::PubRec(PubRec::new(2)),
        Packet::PubRel(PubRel::new(3)),
        Packet::PubComp(PubComp::new(4)),
        Packet::Subscribe(Subscribe {
            packet_id: 5,
            subscriptions: vec![Subscription {
                filter: "topic/#".to_string(),
                options: SubscriptionOptions::default(),
            }],
            properties: Properties::default(),
        }),
        Packet::Unsubscribe(Unsubscribe {
            packet_id: 6,
            filters: vec!["topic/#".to_string()],
            properties: Properties::default(),
        }),
        Packet::PingReq,
        Packet::PingResp,
        Packet::Disconnect(Disconnect::default()),
    ];

    for packet in &packets {
        let encoded = encode_packet(packet, ProtocolVersion::V311);
        let decoded = decode_packet(&encoded, Some(ProtocolVersion::V311)).unwrap();
        assert_eq!(*packet, decoded, "Round-trip failed for {:?}", packet);
    }
}

#[test]
fn test_roundtrip_all_packet_types_v5() {
    let packets = [
        Packet::Connect(Box::new(Connect {
            protocol_version: ProtocolVersion::V5,
            client_id: "test".to_string(),
            clean_start: true,
            keep_alive: 60,
            username: None,
            password: None,
            will: None,
            properties: Properties::default(),
        })),
        Packet::ConnAck(ConnAck {
            session_present: false,
            reason_code: ReasonCode::Success,
            properties: Properties::default(),
        }),
        Packet::Publish(Publish {
            dup: false,
            qos: QoS::ExactlyOnce,
            retain: true,
            topic: "test".to_string(),
            packet_id: Some(1),
            payload: Bytes::from("data"),
            properties: Properties::default(),
        }),
        Packet::PubAck(PubAck::new(1)),
        Packet::PubRec(PubRec::new(2)),
        Packet::PubRel(PubRel::new(3)),
        Packet::PubComp(PubComp::new(4)),
        Packet::Subscribe(Subscribe {
            packet_id: 5,
            subscriptions: vec![Subscription {
                filter: "topic/#".to_string(),
                options: SubscriptionOptions {
                    qos: QoS::ExactlyOnce,
                    no_local: true,
                    retain_as_published: true,
                    retain_handling: RetainHandling::SendAtSubscribeIfNew,
                },
            }],
            properties: Properties::default(),
        }),
        Packet::SubAck(SubAck {
            packet_id: 5,
            reason_codes: vec![ReasonCode::GrantedQoS2],
            properties: Properties::default(),
        }),
        Packet::Unsubscribe(Unsubscribe {
            packet_id: 6,
            filters: vec!["topic/#".to_string()],
            properties: Properties::default(),
        }),
        Packet::UnsubAck(UnsubAck {
            packet_id: 6,
            reason_codes: vec![ReasonCode::Success],
            properties: Properties::default(),
        }),
        Packet::PingReq,
        Packet::PingResp,
        Packet::Disconnect(Disconnect {
            reason_code: ReasonCode::Success,
            properties: Properties::default(),
        }),
        Packet::Auth(Auth {
            reason_code: ReasonCode::Success,
            properties: Properties::default(),
        }),
    ];

    for packet in &packets {
        let encoded = encode_packet(packet, ProtocolVersion::V5);
        let decoded = decode_packet(&encoded, Some(ProtocolVersion::V5)).unwrap();
        assert_eq!(*packet, decoded, "Round-trip failed for {:?}", packet);
    }
}

// ============================================================================
// Property-Based Tests (using proptest)
// ============================================================================

mod proptest_tests {
    use super::*;
    use crate::codec::{read_string, read_variable_int, write_string, write_variable_int};
    use proptest::prelude::*;

    // Strategy for generating valid MQTT strings (no null chars, valid UTF-8)
    fn mqtt_string_strategy() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9_\\-/]{0,100}".prop_map(|s| s)
    }

    // Strategy for generating valid topic filters
    fn topic_filter_strategy() -> impl Strategy<Value = String> {
        prop_oneof![
            // Simple topics
            "[a-zA-Z0-9]{1,20}(/[a-zA-Z0-9]{1,10}){0,5}",
            // Single-level wildcard
            "[a-zA-Z0-9]{1,10}/\\+(/[a-zA-Z0-9]{1,10}){0,3}",
            // Multi-level wildcard
            "[a-zA-Z0-9]{1,10}(/#)?",
        ]
    }

    // Strategy for generating valid client IDs (1-23 chars, alphanumeric)
    fn client_id_strategy() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9]{1,23}"
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        // Variable byte integer roundtrip
        #[test]
        fn prop_variable_int_roundtrip(value in 0u32..268_435_455u32) {
            let mut buf = BytesMut::new();
            let written = write_variable_int(&mut buf, value).unwrap();
            let (decoded, consumed) = read_variable_int(&buf).unwrap();
            prop_assert_eq!(value, decoded);
            prop_assert_eq!(written, consumed);
        }

        // Variable int encoding length
        #[test]
        fn prop_variable_int_length(value in 0u32..268_435_455u32) {
            let mut buf = BytesMut::new();
            let written = write_variable_int(&mut buf, value).unwrap();
            let expected_len = if value < 128 { 1 }
                else if value < 16_384 { 2 }
                else if value < 2_097_152 { 3 }
                else { 4 };
            prop_assert_eq!(written, expected_len);
        }

        // String encoding roundtrip
        #[test]
        fn prop_string_roundtrip(s in mqtt_string_strategy()) {
            let mut buf = BytesMut::new();
            write_string(&mut buf, &s).unwrap();
            let (decoded, consumed) = read_string(&buf).unwrap();
            prop_assert_eq!(&s, decoded);
            prop_assert_eq!(consumed, 2 + s.len());
        }

        // CONNECT packet roundtrip
        #[test]
        fn prop_connect_roundtrip(
            client_id in client_id_strategy(),
            clean_start in any::<bool>(),
            keep_alive in 0u16..65535u16,
        ) {
            let packet = Packet::Connect(Box::new(Connect {
                protocol_version: ProtocolVersion::V311,
                client_id,
                clean_start,
                keep_alive,
                username: None,
                password: None,
                will: None,
                properties: Properties::default(),
            }));
            let encoded = encode_packet(&packet, ProtocolVersion::V311);
            let decoded = decode_packet(&encoded, None).unwrap();
            prop_assert_eq!(packet, decoded);
        }

        // PUBLISH packet roundtrip with random payloads
        #[test]
        fn prop_publish_roundtrip(
            topic in "[a-zA-Z0-9]{1,50}",
            payload in prop::collection::vec(any::<u8>(), 0..1000),
            retain in any::<bool>(),
        ) {
            let packet = Packet::Publish(Publish {
                dup: false,
                qos: QoS::AtMostOnce,
                retain,
                topic,
                packet_id: None,
                payload: Bytes::from(payload),
                properties: Properties::default(),
            });
            let encoded = encode_packet(&packet, ProtocolVersion::V311);
            let decoded = decode_packet(&encoded, Some(ProtocolVersion::V311)).unwrap();
            prop_assert_eq!(packet, decoded);
        }

        // PUBLISH with QoS 1/2 requires packet_id
        #[test]
        fn prop_publish_qos1_roundtrip(
            topic in "[a-zA-Z0-9]{1,50}",
            payload in prop::collection::vec(any::<u8>(), 0..500),
            packet_id in 1u16..65535u16,
            retain in any::<bool>(),
            dup in any::<bool>(),
        ) {
            let packet = Packet::Publish(Publish {
                dup,
                qos: QoS::AtLeastOnce,
                retain,
                topic,
                packet_id: Some(packet_id),
                payload: Bytes::from(payload),
                properties: Properties::default(),
            });
            let encoded = encode_packet(&packet, ProtocolVersion::V311);
            let decoded = decode_packet(&encoded, Some(ProtocolVersion::V311)).unwrap();
            prop_assert_eq!(packet, decoded);
        }

        // SUBSCRIBE roundtrip with multiple topics
        #[test]
        fn prop_subscribe_roundtrip(
            packet_id in 1u16..65535u16,
            topics in prop::collection::vec(topic_filter_strategy(), 1..5),
        ) {
            let subscriptions: Vec<Subscription> = topics.into_iter().map(|filter| {
                Subscription {
                    filter,
                    options: SubscriptionOptions {
                        qos: QoS::AtMostOnce,
                        no_local: false,
                        retain_as_published: false,
                        retain_handling: RetainHandling::SendAtSubscribe,
                    },
                }
            }).collect();

            let packet = Packet::Subscribe(Subscribe {
                packet_id,
                subscriptions,
                properties: Properties::default(),
            });
            let encoded = encode_packet(&packet, ProtocolVersion::V311);
            let decoded = decode_packet(&encoded, Some(ProtocolVersion::V311)).unwrap();
            prop_assert_eq!(packet, decoded);
        }

        // UNSUBSCRIBE roundtrip
        #[test]
        fn prop_unsubscribe_roundtrip(
            packet_id in 1u16..65535u16,
            topics in prop::collection::vec(topic_filter_strategy(), 1..5),
        ) {
            let packet = Packet::Unsubscribe(Unsubscribe {
                packet_id,
                filters: topics,
                properties: Properties::default(),
            });
            let encoded = encode_packet(&packet, ProtocolVersion::V311);
            let decoded = decode_packet(&encoded, Some(ProtocolVersion::V311)).unwrap();
            prop_assert_eq!(packet, decoded);
        }

        // Fuzzing: random bytes should either decode or return error, never panic
        #[test]
        fn prop_random_bytes_no_panic(data in prop::collection::vec(any::<u8>(), 0..500)) {
            let mut decoder = Decoder::new();
            // This should not panic regardless of input
            let _ = decoder.decode(&data);
        }

        // Fuzzing: corrupted valid packets should handle gracefully
        #[test]
        fn prop_corrupted_packet_no_panic(
            client_id in client_id_strategy(),
            corruption_pos in 0usize..100usize,
            corruption_byte in any::<u8>(),
        ) {
            let packet = Packet::Connect(Box::new(Connect {
                protocol_version: ProtocolVersion::V311,
                client_id,
                clean_start: true,
                keep_alive: 60,
                username: None,
                password: None,
                will: None,
                properties: Properties::default(),
            }));
            let mut encoded = encode_packet(&packet, ProtocolVersion::V311);

            // Corrupt a byte if within range
            if corruption_pos < encoded.len() {
                encoded[corruption_pos] = corruption_byte;
            }

            // Should not panic
            let mut decoder = Decoder::new();
            let _ = decoder.decode(&encoded);
        }
    }
}

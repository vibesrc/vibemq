//! MQTT Conformance Tests
//!
//! Tests organized by MQTT version and specification section.
//! Each test references its normative requirement(s) in the function name and comments.
//!
//! Structure:
//! - conformance/v3_1_1/ - MQTT v3.1.1 conformance tests
//! - conformance/v5/ - MQTT v5.0 conformance tests
//!
//! Test naming convention: test_mqtt_<ref>_<description>
//! Example: test_mqtt_3_1_0_1_first_packet_must_be_connect
//!
//! Coverage Status (v3.1.1):
//! ========================
//! Section 1.5 - Data Representations:
//!   [x] MQTT-1.5.3-1: Ill-formed UTF-8 closes connection
//!   [x] MQTT-1.5.3-2: Null character U+0000 closes connection
//!
//! Section 2.3 - Packet Identifier:
//!   [x] MQTT-2.3.1-1: Non-zero packet ID for QoS>0
//!   [x] MQTT-2.3.1-6: PUBACK/PUBREC/PUBCOMP same packet ID
//!   [x] MQTT-2.3.1-7: SUBACK/UNSUBACK same packet ID
//!
//! Section 3.1 - CONNECT:
//!   [x] MQTT-3.1.0-1: First packet must be CONNECT
//!   [x] MQTT-3.1.0-2: Second CONNECT is protocol violation
//!   [x] MQTT-3.1.2-1: Invalid protocol name
//!   [x] MQTT-3.1.2-2: Unsupported protocol version
//!   [x] MQTT-3.1.2-3: Reserved flag must be zero
//!   [x] MQTT-3.1.2-4/5/6: CleanSession behavior
//!   [x] MQTT-3.1.2-8: Will message published on abnormal disconnect
//!   [x] MQTT-3.1.2-24: Keep alive timeout (1.5x)
//!   [x] MQTT-3.1.3-8: Empty client ID with CleanSession=0 rejected
//!   [x] MQTT-3.1.4-2: Session takeover disconnects existing client
//!
//! Section 3.2 - CONNACK:
//!   [x] MQTT-3.2.2-1/2/3: Session present flag
//!
//! Section 3.3 - PUBLISH:
//!   [x] MQTT-3.3.1-2: DUP must be 0 for QoS 0
//!   [x] MQTT-3.3.1-4: QoS 3 is invalid
//!   [x] MQTT-3.3.1-5: Retained message storage
//!   [x] MQTT-3.3.1-8/9: Retain flag on subscription delivery
//!   [x] MQTT-3.3.1-10/11: Empty retained clears existing
//!   [x] MQTT-3.3.2-2: Wildcard in PUBLISH topic invalid
//!
//! Section 3.6 - PUBREL:
//!   [x] MQTT-3.6.1-1: PUBREL flags must be 0010
//!
//! Section 3.8 - SUBSCRIBE:
//!   [x] MQTT-3.8.1-1: SUBSCRIBE flags must be 0010
//!   [x] MQTT-3.8.3-3: At least one topic filter
//!   [x] MQTT-3.8.4-3: Subscription replacement
//!
//! Section 3.10 - UNSUBSCRIBE:
//!   [x] MQTT-3.10.1-1: UNSUBSCRIBE flags must be 0010
//!   [x] MQTT-3.10.3-2: At least one topic filter
//!   [x] MQTT-3.10.4-1/2: UNSUBACK response
//!
//! Section 3.12 - PINGREQ:
//!   [x] MQTT-3.12.4-1: Server responds with PINGRESP
//!
//! Section 3.14 - DISCONNECT:
//!   [x] MQTT-3.14.4-2: Will message discarded on normal disconnect
//!
//! Section 4.7 - Topics:
//!   [x] MQTT-4.7.1-1/2: Wildcard validation
//!   [x] MQTT-4.7.2-1: $ topic matching
//!   [x] MQTT-4.7.3-1: No wildcards in topic names

mod mqtt_conformance;

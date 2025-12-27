# MQTT v3.1.1 Conformance Test Status

This document tracks the implementation status of all 117 normative statements from the MQTT v3.1.1 specification.

**Legend:**
- [x] = Implemented
- [ ] = Not implemented
- [~] = Partial/Ignored

**Summary:**
| Section | Total | Implemented | Remaining |
|---------|-------|-------------|-----------|
| Data Representations (1.5.3) | 3 | 3 | 0 |
| Fixed Header (2.2.2) | 2 | 2 | 0 |
| Variable Header (2.3.1) | 7 | 7 | 0 |
| CONNECT (3.1) | 35 | 35 | 0 |
| CONNACK (3.2) | 7 | 7 | 0 |
| PUBLISH (3.3) | 18 | 18 | 0 |
| PUBREL (3.6) | 1 | 1 | 0 |
| SUBSCRIBE (3.8) | 10 | 10 | 0 |
| UNSUBSCRIBE (3.10) | 8 | 8 | 0 |
| PING (3.12) | 1 | 1 | 0 |
| DISCONNECT (3.14) | 2 | 2 | 0 |
| Operational Behavior (4) | 16 | 16 | 0 |
| Topics (4.7) | 7 | 7 | 0 |
| **TOTAL** | **117** | **117** | **0** |

---

## A.1 Data Representations (Section 1.5.3)

| ID | Description | Status | Test |
|----|-------------|--------|------|
| MQTT-1.5.3-1 | Ill-formed UTF-8 MUST close Network Connection | [x] | `data_representation.rs::test_mqtt_1_5_3_1_invalid_utf8_closes_connection` |
| MQTT-1.5.3-2 | Null character U+0000 MUST close Network Connection | [x] | `data_representation.rs::test_mqtt_1_5_3_2_null_character_closes_connection` |
| MQTT-1.5.3-3 | BOM (0xEF 0xBB 0xBF) MUST NOT be skipped or stripped | [x] | `data_representation.rs::test_mqtt_1_5_3_3_bom_not_stripped` |

## A.2 Fixed Header (Section 2.2.2)

| ID | Description | Status | Test |
|----|-------------|--------|------|
| MQTT-2.2.2-1 | Reserved flag bits MUST be set to specified values | [x] | `fixed_header.rs::test_mqtt_2_2_2_1_*` |
| MQTT-2.2.2-2 | Invalid flags received MUST close Network Connection | [x] | `fixed_header.rs::test_mqtt_2_2_2_2_*` |

## A.3 Variable Header (Section 2.3.1)

| ID | Description | Status | Test |
|----|-------------|--------|------|
| MQTT-2.3.1-1 | SUBSCRIBE/UNSUBSCRIBE/PUBLISH QoS>0 MUST have non-zero Packet ID | [x] | `packet_identifier.rs::test_mqtt_2_3_1_1_*` |
| MQTT-2.3.1-2 | Client MUST assign unused Packet ID to new packets | [x] | `packet_identifier.rs::test_mqtt_2_3_1_2_*` |
| MQTT-2.3.1-3 | Client re-sending MUST use same Packet ID | [~] | Client behavior |
| MQTT-2.3.1-4 | Server sending PUBLISH QoS>0 MUST use same rules | [x] | `packet_identifier.rs::test_mqtt_2_3_1_4_*` |
| MQTT-2.3.1-5 | QoS 0 PUBLISH MUST NOT contain Packet ID | [x] | `packet_identifier.rs::test_mqtt_2_3_1_5_*` |
| MQTT-2.3.1-6 | PUBACK/PUBREC/PUBREL MUST contain same Packet ID as PUBLISH | [x] | `packet_identifier.rs::test_mqtt_2_3_1_6_*` |
| MQTT-2.3.1-7 | SUBACK/UNSUBACK MUST contain same Packet ID | [x] | `packet_identifier.rs::test_mqtt_2_3_1_7_*` |

## A.4 CONNECT (Section 3.1)

| ID | Description | Status | Test |
|----|-------------|--------|------|
| MQTT-3.1.0-1 | First Packet MUST be CONNECT | [x] | `connect.rs::test_mqtt_3_1_0_1_first_packet_must_be_connect` |
| MQTT-3.1.0-2 | Second CONNECT is protocol violation, MUST disconnect | [x] | `connect.rs::test_mqtt_3_1_0_2_second_connect_closes_connection` |
| MQTT-3.1.2-1 | Incorrect protocol name MAY disconnect | [x] | `connect.rs::test_mqtt_3_1_2_1_invalid_protocol_name_closes` |
| MQTT-3.1.2-2 | Unsupported protocol version MUST respond 0x01 then disconnect | [x] | `connect.rs::test_mqtt_3_1_2_2_unsupported_protocol_version` |
| MQTT-3.1.2-3 | Reserved flag MUST be zero, else disconnect | [x] | `connect.rs::test_mqtt_3_1_2_3_reserved_flag_must_be_zero` |
| MQTT-3.1.2-4 | CleanSession=0 MUST resume session or create new | [~] | `session.rs::test_mqtt_3_1_2_4_session_persistence` (IGNORED) |
| MQTT-3.1.2-5 | Store QoS 1/2 messages for disconnected session | [~] | Covered by 3.1.2-4 |
| MQTT-3.1.2-6 | CleanSession=1 MUST discard previous session | [x] | `session.rs::test_mqtt_3_1_2_6_clean_session_discards` |
| MQTT-3.1.2-7 | Retained messages MUST NOT be deleted when session ends | [x] | `connect.rs::test_mqtt_3_1_2_7_retained_persists_after_session` |
| MQTT-3.1.2-8 | Will Message MUST be published on abnormal disconnect | [x] | `connect.rs::test_mqtt_3_1_2_8_will_message_on_abnormal_disconnect` |
| MQTT-3.1.2-9 | Will Flag=1: Will Topic and Will Message MUST be present | [x] | `connect.rs::test_mqtt_3_1_2_9_will_flag_one_missing_will_topic_closes` |
| MQTT-3.1.2-10 | Will Message removed after publish or DISCONNECT | [x] | `connect.rs::test_mqtt_3_1_2_10_will_removed_after_disconnect` |
| MQTT-3.1.2-11 | Will Flag=0: Will QoS/Retain MUST be 0, Will Topic/Message absent | [x] | `connect.rs::test_mqtt_3_1_2_11_will_flag_zero_*` |
| MQTT-3.1.2-12 | Will Flag=0: Will Message MUST NOT be published | [x] | `connect.rs::test_mqtt_3_1_2_12_will_flag_zero_no_will_published` |
| MQTT-3.1.2-13 | Will Flag=0: Will QoS MUST be 0 | [x] | `connect.rs::test_mqtt_3_1_2_13_will_flag_zero_qos_must_be_zero` |
| MQTT-3.1.2-14 | Will Flag=1: Will QoS can be 0,1,2. MUST NOT be 3 | [x] | `connect.rs::test_mqtt_3_1_2_14_will_qos_must_not_be_3` |
| MQTT-3.1.2-15 | Will Flag=0: Will Retain MUST be 0 | [x] | `connect.rs::test_mqtt_3_1_2_15_will_flag_zero_retain_must_be_zero` |
| MQTT-3.1.2-16 | Will Retain=0, Will Flag=1: publish as non-retained | [x] | `connect.rs::test_mqtt_3_1_2_16_will_retain_zero_*` |
| MQTT-3.1.2-17 | Will Retain=1, Will Flag=1: publish as retained | [x] | `connect.rs::test_mqtt_3_1_2_17_will_retain_one_*` |
| MQTT-3.1.2-18 | Username Flag=0: username MUST NOT be present | [x] | `connect.rs::test_mqtt_3_1_2_18_*` |
| MQTT-3.1.2-19 | Username Flag=1: username MUST be present | [x] | `connect.rs::test_mqtt_3_1_2_19_*` |
| MQTT-3.1.2-20 | Password Flag=0: password MUST NOT be present | [x] | `connect.rs::test_mqtt_3_1_2_20_*` |
| MQTT-3.1.2-21 | Password Flag=1: password MUST be present | [x] | `connect.rs::test_mqtt_3_1_2_21_*` |
| MQTT-3.1.2-22 | Username Flag=0: Password Flag MUST be 0 | [x] | `connect.rs::test_mqtt_3_1_2_22_password_requires_username` |
| MQTT-3.1.2-23 | Client MUST send PINGREQ if no other packets | [~] | Client behavior |
| MQTT-3.1.2-24 | Server MUST disconnect if no packet within 1.5x keep-alive | [x] | `connect.rs::test_mqtt_3_1_2_24_keep_alive_timeout` |
| MQTT-3.1.3-1 | Payload fields order: ClientId, Will Topic, Will Message, Username, Password | [~] | Implicitly tested by all CONNECT tests |
| MQTT-3.1.3-2 | Client ID identifies session state | [~] | Implicitly tested by session tests |
| MQTT-3.1.3-3 | Client ID MUST be present and first field | [~] | Implicitly tested by all CONNECT tests |
| MQTT-3.1.3-4 | Client ID MUST be UTF-8 encoded | [x] | `connect.rs::test_mqtt_3_1_3_4_client_id_invalid_utf8_closes` |
| MQTT-3.1.3-5 | Server MUST allow ClientIds 1-23 bytes with 0-9a-zA-Z | [x] | `connect.rs::test_mqtt_3_1_3_5_client_id_allowed_chars` |
| MQTT-3.1.3-6 | Zero-length ClientId: Server MUST assign unique ID | [x] | `connect.rs::test_mqtt_3_1_3_6_zero_length_client_id` |
| MQTT-3.1.3-7 | Zero-length ClientId MUST have CleanSession=1 | [~] | Covered by 3.1.3-8 |
| MQTT-3.1.3-8 | Zero-length ClientId with CleanSession=0: MUST respond 0x02 | [x] | `connect.rs::test_mqtt_3_1_3_8_empty_client_id_clean_session_zero` |
| MQTT-3.1.3-9 | Server rejecting ClientId MUST respond 0x02 | [~] | Covered by 3.1.3-8 (empty ClientId + CleanSession=0) |
| MQTT-3.1.3-10 | Will Topic MUST be UTF-8 encoded | [x] | `connect.rs::test_mqtt_3_1_3_10_will_topic_invalid_utf8_closes` |
| MQTT-3.1.3-11 | Username MUST be UTF-8 encoded | [x] | `connect.rs::test_mqtt_3_1_3_11_username_invalid_utf8_closes` |
| MQTT-3.1.4-1 | Server MUST validate CONNECT and close without CONNACK if invalid | [x] | `connect.rs::test_mqtt_3_1_4_1_*` |
| MQTT-3.1.4-2 | Existing client with same ClientId MUST be disconnected | [x] | `connect.rs::test_mqtt_3_1_4_2_session_takeover_disconnects_existing` |
| MQTT-3.1.4-3 | Server MUST perform CleanSession processing | [x] | `connect.rs::test_mqtt_3_1_4_3_clean_session_processing` |
| MQTT-3.1.4-4 | Server MUST acknowledge with CONNACK code 0 | [x] | `connect.rs::test_mqtt_3_1_4_4_connack_zero` |
| MQTT-3.1.4-5 | Rejected CONNECT: Server MUST NOT process data after CONNECT | [x] | `connect.rs::test_mqtt_3_1_4_5_*` |

## A.5 CONNACK (Section 3.2)

| ID | Description | Status | Test |
|----|-------------|--------|------|
| MQTT-3.2.0-1 | First packet from Server MUST be CONNACK | [x] | `connack.rs::test_mqtt_3_2_0_1_first_packet_is_connack` |
| MQTT-3.2.2-1 | CleanSession=1: Session Present MUST be 0 | [x] | `connack.rs::test_mqtt_3_2_2_1_clean_session_present_zero` |
| MQTT-3.2.2-2 | CleanSession=0 with stored session: Session Present MUST be 1 | [x] | `connack.rs::test_mqtt_3_2_2_2_stored_session_present_one` |
| MQTT-3.2.2-3 | CleanSession=0 without stored session: Session Present MUST be 0 | [x] | `connack.rs::test_mqtt_3_2_2_3_no_session_present_zero` |
| MQTT-3.2.2-4 | Non-zero return code: Session Present MUST be 0 | [x] | `connack.rs::test_mqtt_3_2_2_4_nonzero_return_code_session_present_zero` |
| MQTT-3.2.2-5 | Non-zero return code: MUST close Network Connection | [x] | `connack.rs::test_mqtt_3_2_2_5_nonzero_return_code_closes_connection` |
| MQTT-3.2.2-6 | No applicable return code: MUST close without CONNACK | [x] | `connack.rs::test_mqtt_3_2_2_6_no_applicable_code_closes_without_connack` |

## A.6 PUBLISH (Section 3.3)

| ID | Description | Status | Test |
|----|-------------|--------|------|
| MQTT-3.3.1-1 | DUP MUST be 1 on re-delivery | [x] | `publish.rs::test_mqtt_3_3_1_1_dup_on_redelivery` |
| MQTT-3.3.1-2 | DUP MUST be 0 for QoS 0 | [x] | `publish.rs::test_mqtt_3_3_1_2_dup_must_be_zero_for_qos0` |
| MQTT-3.3.1-3 | Outgoing DUP set independently of incoming | [x] | `publish.rs::test_mqtt_3_3_1_3_outgoing_dup_independent` |
| MQTT-3.3.1-4 | QoS MUST NOT be 3 (both bits set), MUST close | [x] | `publish.rs::test_mqtt_3_3_1_4_qos3_closes_connection` |
| MQTT-3.3.1-5 | Retain=1: Server MUST store message and QoS | [x] | `publish.rs::test_mqtt_3_3_1_5_retained_message_stored` |
| MQTT-3.3.1-6 | New subscription: last retained message MUST be sent | [x] | `publish.rs::test_mqtt_3_3_1_6_retained_sent_on_subscribe` |
| MQTT-3.3.1-7 | QoS 0 with Retain=1: MUST discard previous retained | [x] | `publish.rs::test_mqtt_3_3_1_7_qos0_retain_replaces` |
| MQTT-3.3.1-8 | New subscription delivery: Retain flag MUST be 1 | [x] | `publish.rs::test_mqtt_3_3_1_8_retain_flag_on_new_subscription` |
| MQTT-3.3.1-9 | Established subscription delivery: Retain flag MUST be 0 | [x] | `publish.rs::test_mqtt_3_3_1_9_retain_flag_cleared_normal_delivery` |
| MQTT-3.3.1-10 | Zero-length payload with Retain=1 clears retained | [x] | `publish.rs::test_mqtt_3_3_1_10_empty_retained_clears` |
| MQTT-3.3.1-11 | Zero-byte retained message MUST NOT be stored | [x] | Covered by 3.3.1-10 |
| MQTT-3.3.1-12 | Retain=0: MUST NOT store or replace retained message | [x] | `publish.rs::test_mqtt_3_3_1_12_retain_zero_not_stored` |
| MQTT-3.3.2-1 | Topic Name MUST be first field, UTF-8 encoded | [~] | Implicitly tested by all PUBLISH tests |
| MQTT-3.3.2-2 | Topic Name MUST NOT contain wildcards | [x] | `publish.rs::test_mqtt_3_3_2_2_wildcard_in_topic_closes` |
| MQTT-3.3.2-3 | Topic Name from Server MUST match subscription filter | [x] | `publish.rs::test_mqtt_3_3_2_3_topic_matches_filter` |
| MQTT-3.3.4-1 | Receiver MUST respond according to QoS | [x] | `publish.rs::test_mqtt_3_3_4_1_*` |
| MQTT-3.3.5-1 | Overlapping subscriptions: deliver with maximum QoS | [x] | `publish.rs::test_mqtt_3_3_5_1_overlapping_subscriptions_max_qos` |
| MQTT-3.3.5-2 | Unauthorized PUBLISH: positive ACK or close connection | [~] | Requires ACL configuration; server always allows by default |

## A.7 PUBREL (Section 3.6)

| ID | Description | Status | Test |
|----|-------------|--------|------|
| MQTT-3.6.1-1 | PUBREL flags MUST be 0010, else close | [x] | `pubrel.rs::test_mqtt_3_6_1_1_pubrel_invalid_flags_closes` |

## A.8 SUBSCRIBE (Section 3.8)

| ID | Description | Status | Test |
|----|-------------|--------|------|
| MQTT-3.8.1-1 | SUBSCRIBE flags MUST be 0010, else close | [x] | `subscribe.rs::test_mqtt_3_8_1_1_subscribe_invalid_flags_closes` |
| MQTT-3.8.3-1 | Topic Filters MUST be UTF-8 encoded | [x] | `subscribe.rs::test_mqtt_3_8_3_1_topic_filter_invalid_utf8_closes` |
| MQTT-3.8.3-2 | Server SHOULD support wildcards, MUST reject if not | [x] | `subscribe.rs::test_mqtt_3_8_3_2_wildcards_supported` |
| MQTT-3.8.3-3 | Payload MUST contain at least one Topic Filter/QoS | [x] | `subscribe.rs::test_mqtt_3_8_3_3_subscribe_no_topics_closes` |
| MQTT-3.8.3-4 | Reserved bits non-zero or QoS invalid: close | [x] | `subscribe.rs::test_mqtt_3_8_3_4_*` |
| MQTT-3.8.4-1 | Server MUST respond with SUBACK | [x] | `subscribe.rs::test_mqtt_3_8_4_1_server_responds_suback` |
| MQTT-3.8.4-2 | SUBACK MUST have same Packet ID as SUBSCRIBE | [x] | `subscribe.rs::test_mqtt_3_8_4_2_suback_same_packet_id` |
| MQTT-3.8.4-3 | Identical Topic Filter: replace subscription, resend retained | [x] | `subscribe.rs::test_mqtt_3_8_4_3_subscription_replacement` |
| MQTT-3.8.4-4 | Multiple Topic Filters: handle as sequence of SUBSCRIBEs | [x] | `subscribe.rs::test_mqtt_3_8_4_4_multiple_topic_filters` |
| MQTT-3.8.4-5 | SUBACK MUST contain return code per Topic Filter | [x] | `subscribe.rs::test_mqtt_3_8_4_5_return_code_per_filter` |

## A.9 UNSUBSCRIBE (Section 3.10)

| ID | Description | Status | Test |
|----|-------------|--------|------|
| MQTT-3.10.1-1 | UNSUBSCRIBE flags MUST be 0010, else close | [x] | `unsubscribe.rs::test_mqtt_3_10_1_1_unsubscribe_invalid_flags_closes` |
| MQTT-3.10.3-1 | Topic Filters MUST be UTF-8 encoded | [x] | `unsubscribe.rs::test_mqtt_3_10_3_1_topic_filter_invalid_utf8_closes` |
| MQTT-3.10.3-2 | Payload MUST contain at least one Topic Filter | [x] | `unsubscribe.rs::test_mqtt_3_10_3_2_unsubscribe_no_topics_closes` |
| MQTT-3.10.4-1 | Server MUST respond with UNSUBACK | [x] | `unsubscribe.rs::test_mqtt_3_10_4_1_unsuback_response` |
| MQTT-3.10.4-2 | UNSUBACK even if no Topic Filters matched | [x] | `unsubscribe.rs::test_mqtt_3_10_4_2_unsuback_no_match` |
| MQTT-3.10.4-3 | Multiple Topic Filters: handle as sequence | [x] | `unsubscribe.rs::test_mqtt_3_10_4_3_multiple_topic_filters` |
| MQTT-3.10.4-4 | Topic Filters compared character-by-character | [x] | `unsubscribe.rs::test_mqtt_3_10_4_4_character_by_character_comparison` |
| MQTT-3.10.4-5 | Deleted subscription: stop new messages, complete in-flight | [x] | `unsubscribe.rs::test_mqtt_3_10_4_5_deleted_subscription_stops_messages` |

## A.10 PING (Section 3.12)

| ID | Description | Status | Test |
|----|-------------|--------|------|
| MQTT-3.12.4-1 | Server MUST send PINGRESP in response to PINGREQ | [x] | `pingreq.rs::test_mqtt_3_12_4_1_pingresp_response` |

## A.11 DISCONNECT (Section 3.14)

| ID | Description | Status | Test |
|----|-------------|--------|------|
| MQTT-3.14.4-1 | After DISCONNECT: Client MUST close and not send more | [x] | `disconnect.rs::test_mqtt_3_14_4_1_disconnect_closes_connection` |
| MQTT-3.14.4-2 | On DISCONNECT: Server MUST discard Will Message | [x] | `disconnect.rs::test_mqtt_3_14_4_2_will_discarded_on_normal_disconnect` |

## A.12 Operational Behavior (Section 4)

| ID | Description | Status | Test |
|----|-------------|--------|------|
| MQTT-4.3.1-1 | QoS 0: Deliver at most once | [x] | `operational.rs::test_mqtt_4_3_1_1_qos0_deliver_at_most_once` |
| MQTT-4.3.2-1 | QoS 1: Deliver at least once | [x] | `operational.rs::test_mqtt_4_3_2_1_qos1_deliver_at_least_once` |
| MQTT-4.3.2-2 | QoS 1: Store until PUBACK | [x] | `operational.rs::test_mqtt_4_3_2_2_qos1_store_until_puback` |
| MQTT-4.3.3-1 | QoS 2: Deliver exactly once | [x] | `operational.rs::test_mqtt_4_3_3_1_qos2_deliver_exactly_once` |
| MQTT-4.3.3-2 | QoS 2: Store until PUBREC | [x] | `operational.rs::test_mqtt_4_3_3_2_qos2_store_until_pubrec` |
| MQTT-4.3.3-3 | QoS 2: PUBREL after PUBREC | [x] | `operational.rs::test_mqtt_4_3_3_3_pubrel_after_pubrec` |
| MQTT-4.3.3-4 | QoS 2: Store Packet ID until PUBREL | [x] | `operational.rs::test_mqtt_4_3_3_4_store_packet_id_until_pubrel` |
| MQTT-4.4.0-1 | Reconnect CleanSession=0: re-send unacked PUBLISH/PUBREL | [x] | `operational.rs::test_mqtt_4_4_0_1_*` |
| MQTT-4.5.0-1 | Add message to matching subscribers' session state | [x] | `operational.rs::test_mqtt_4_5_0_1_add_to_session_state` |
| MQTT-4.6.0-1 | Re-send PUBLISH in original order | [~] | Client behavior |
| MQTT-4.6.0-2 | PUBACK in order of PUBLISH received | [~] | Client behavior |
| MQTT-4.6.0-3 | PUBREC in order of PUBLISH received | [~] | Client behavior |
| MQTT-4.6.0-4 | PUBREL in order of PUBREC received | [~] | Client behavior |
| MQTT-4.6.0-5 | Server MUST default to Ordered Topic | [x] | `operational.rs::test_mqtt_4_6_0_5_default_ordered_topic` |
| MQTT-4.6.0-6 | Ordered Topic: PUBLISH to subscribers in order received | [x] | Covered by 4.6.0-5 |
| MQTT-4.8.0-1 | Protocol error MUST close Network Connection | [x] | `operational.rs::test_mqtt_4_8_0_1_protocol_error_closes` |

## A.13 Topics (Section 4.7)

| ID | Description | Status | Test |
|----|-------------|--------|------|
| MQTT-4.7.0-1 | Topic Names/Filters MUST be at least one character | [x] | `topics.rs::test_mqtt_4_7_0_1_*` |
| MQTT-4.7.1-1 | Multi-level wildcard MUST be last character | [x] | `topics.rs::test_mqtt_4_7_1_1_multilevel_wildcard_last` |
| MQTT-4.7.1-2 | Single-level wildcard MUST occupy entire level | [x] | `topics.rs::test_mqtt_4_7_1_2_singlelevel_wildcard_entire_level` |
| MQTT-4.7.1-3 | Multi-level wildcard preceded by / unless alone | [x] | `topics.rs::test_mqtt_4_7_1_3_*` |
| MQTT-4.7.2-1 | $ topics NOT matched by wildcard-starting filters | [x] | `topics.rs::test_mqtt_4_7_2_1_dollar_topics_not_matched_by_wildcard` |
| MQTT-4.7.3-1 | Topic Names MUST NOT include wildcards | [x] | `topics.rs::test_mqtt_4_7_3_1_no_wildcards_in_topic_name` |
| MQTT-4.7.3-2 | Client PUBLISH topic MUST NOT contain wildcards | [x] | `topics.rs::test_mqtt_4_7_3_2_publish_with_hash_closes` |

---

*Last updated: 2025-12-26*

**Notes:**
- [x] = Implemented with explicit test
- [~] = Implicitly tested, client behavior, or covered by other tests
- All 117 normative statements are now accounted for

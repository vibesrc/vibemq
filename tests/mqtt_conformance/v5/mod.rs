//! MQTT v5.0 Conformance Tests
//!
//! Tests organized by specification section number.
//! Each test references normative statements from the MQTT 5.0 spec.

pub mod auth;
pub mod connack;
pub mod connect;
pub mod data_representation;
pub mod disconnect;
pub mod fixed_header;
pub mod message_ordering;
pub mod packet_identifier;
pub mod ping;
pub mod properties;
pub mod publish;
pub mod pubrel;
pub mod shared_subscriptions;
pub mod subscribe;
pub mod topics;
pub mod unsubscribe;

use crate::mqtt_conformance::RawClient;

/// Common MQTT v5.0 CONNECT packet with Clean Start
/// Client ID: "a", Protocol Version 5, Clean Start=1
/// Remaining = 6 (protocol name) + 1 (version) + 1 (flags) + 2 (keep alive) + 1 (props len) + 3 (client id) = 14
pub const CONNECT_V5: [u8; 16] = [
    0x10, 0x0E, // CONNECT, remaining = 14
    0x00, 0x04, b'M', b'Q', b'T', b'T', // Protocol name
    0x05, // Protocol version 5
    0x02, // Connect flags: Clean Start = 1
    0x00, 0x3C, // Keep alive = 60
    0x00, // Properties length = 0
    0x00, 0x01, b'a', // Client ID = "a"
];

/// Build a v5 CONNECT packet with custom options
pub fn build_connect_v5(
    client_id: &str,
    clean_start: bool,
    keep_alive: u16,
    properties: &[u8],
) -> Vec<u8> {
    let flags = if clean_start { 0x02 } else { 0x00 };
    let client_id_bytes = client_id.as_bytes();
    let remaining_len = 10 + properties.len() + 2 + client_id_bytes.len();

    let mut packet = vec![0x10];
    // Encode remaining length (simplified for small packets)
    if remaining_len < 128 {
        packet.push(remaining_len as u8);
    } else {
        packet.push((remaining_len % 128) as u8 | 0x80);
        packet.push((remaining_len / 128) as u8);
    }
    packet.extend_from_slice(&[0x00, 0x04, b'M', b'Q', b'T', b'T', 0x05, flags]);
    packet.extend_from_slice(&[(keep_alive >> 8) as u8, keep_alive as u8]);
    // Properties
    if properties.len() < 128 {
        packet.push(properties.len() as u8);
    } else {
        packet.push((properties.len() % 128) as u8 | 0x80);
        packet.push((properties.len() / 128) as u8);
    }
    packet.extend_from_slice(properties);
    // Client ID
    packet.extend_from_slice(&[
        (client_id_bytes.len() >> 8) as u8,
        client_id_bytes.len() as u8,
    ]);
    packet.extend_from_slice(client_id_bytes);

    packet
}

/// Build a v5 SUBSCRIBE packet
pub fn build_subscribe_v5(
    packet_id: u16,
    topic: &str,
    qos: u8,
    properties: &[u8],
    options: u8,
) -> Vec<u8> {
    let topic_bytes = topic.as_bytes();
    // options byte includes: QoS (bits 0-1), No Local (bit 2), RAP (bit 3), Retain Handling (bits 4-5)
    let sub_options = (options & 0xFC) | (qos & 0x03);
    let remaining_len = 2 + 1 + properties.len() + 2 + topic_bytes.len() + 1;

    let mut packet = vec![0x82]; // SUBSCRIBE
    if remaining_len < 128 {
        packet.push(remaining_len as u8);
    } else {
        packet.push((remaining_len % 128) as u8 | 0x80);
        packet.push((remaining_len / 128) as u8);
    }
    packet.extend_from_slice(&[(packet_id >> 8) as u8, packet_id as u8]);
    // Properties
    if properties.len() < 128 {
        packet.push(properties.len() as u8);
    } else {
        packet.push((properties.len() % 128) as u8 | 0x80);
        packet.push((properties.len() / 128) as u8);
    }
    packet.extend_from_slice(properties);
    // Topic filter
    packet.extend_from_slice(&[(topic_bytes.len() >> 8) as u8, topic_bytes.len() as u8]);
    packet.extend_from_slice(topic_bytes);
    packet.push(sub_options);

    packet
}

/// Build a v5 PUBLISH packet
pub fn build_publish_v5(
    topic: &str,
    payload: &[u8],
    qos: u8,
    retain: bool,
    dup: bool,
    packet_id: Option<u16>,
    properties: &[u8],
) -> Vec<u8> {
    let topic_bytes = topic.as_bytes();
    let mut flags = (qos & 0x03) << 1;
    if retain {
        flags |= 0x01;
    }
    if dup {
        flags |= 0x08;
    }

    let packet_id_len = if qos > 0 { 2 } else { 0 };
    let remaining_len =
        2 + topic_bytes.len() + packet_id_len + 1 + properties.len() + payload.len();

    let mut packet = vec![0x30 | flags];
    if remaining_len < 128 {
        packet.push(remaining_len as u8);
    } else if remaining_len < 16384 {
        packet.push((remaining_len % 128) as u8 | 0x80);
        packet.push((remaining_len / 128) as u8);
    } else {
        packet.push((remaining_len % 128) as u8 | 0x80);
        packet.push(((remaining_len / 128) % 128) as u8 | 0x80);
        packet.push((remaining_len / 16384) as u8);
    }

    // Topic
    packet.extend_from_slice(&[(topic_bytes.len() >> 8) as u8, topic_bytes.len() as u8]);
    packet.extend_from_slice(topic_bytes);

    // Packet ID (if QoS > 0)
    if let Some(pid) = packet_id {
        packet.extend_from_slice(&[(pid >> 8) as u8, pid as u8]);
    }

    // Properties
    if properties.len() < 128 {
        packet.push(properties.len() as u8);
    } else {
        packet.push((properties.len() % 128) as u8 | 0x80);
        packet.push((properties.len() / 128) as u8);
    }
    packet.extend_from_slice(properties);

    // Payload
    packet.extend_from_slice(payload);

    packet
}

/// Build a v5 UNSUBSCRIBE packet
pub fn build_unsubscribe_v5(packet_id: u16, topic: &str, properties: &[u8]) -> Vec<u8> {
    let topic_bytes = topic.as_bytes();
    let remaining_len = 2 + 1 + properties.len() + 2 + topic_bytes.len();

    let mut packet = vec![0xA2]; // UNSUBSCRIBE
    if remaining_len < 128 {
        packet.push(remaining_len as u8);
    } else {
        packet.push((remaining_len % 128) as u8 | 0x80);
        packet.push((remaining_len / 128) as u8);
    }
    packet.extend_from_slice(&[(packet_id >> 8) as u8, packet_id as u8]);
    // Properties
    if properties.len() < 128 {
        packet.push(properties.len() as u8);
    } else {
        packet.push((properties.len() % 128) as u8 | 0x80);
        packet.push((properties.len() / 128) as u8);
    }
    packet.extend_from_slice(properties);
    // Topic filter
    packet.extend_from_slice(&[(topic_bytes.len() >> 8) as u8, topic_bytes.len() as u8]);
    packet.extend_from_slice(topic_bytes);

    packet
}

/// Helper to connect a RawClient with v5
pub async fn connect_v5(client: &mut RawClient) -> Option<Vec<u8>> {
    client.send_raw(&CONNECT_V5).await;
    client.recv_raw(1000).await
}

/// Helper to connect with custom client ID
#[allow(dead_code)]
pub async fn connect_v5_with_id(
    client: &mut RawClient,
    client_id: &str,
    clean_start: bool,
) -> Option<Vec<u8>> {
    let packet = build_connect_v5(client_id, clean_start, 60, &[]);
    client.send_raw(&packet).await;
    client.recv_raw(1000).await
}

/// Extract reason code from CONNACK (v5)
#[allow(dead_code)]
pub fn connack_reason_code(data: &[u8]) -> Option<u8> {
    if data.len() >= 4 && data[0] == 0x20 {
        Some(data[3])
    } else {
        None
    }
}

/// Extract session present flag from CONNACK
#[allow(dead_code)]
pub fn connack_session_present(data: &[u8]) -> Option<bool> {
    if data.len() >= 3 && data[0] == 0x20 {
        Some(data[2] != 0)
    } else {
        None
    }
}

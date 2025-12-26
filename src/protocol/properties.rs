//! MQTT v5.0 Properties
//!
//! Based on spec/v5.0/2.2_variable-header.md

use bytes::{BufMut, Bytes, BytesMut};

use crate::codec::{
    read_binary, read_string, read_variable_int, variable_int_len, write_binary, write_string,
    write_variable_int,
};
use crate::protocol::{DecodeError, EncodeError};

/// Property identifiers as defined in Table 2-4 of the MQTT v5.0 spec
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PropertyId {
    PayloadFormatIndicator = 0x01,
    MessageExpiryInterval = 0x02,
    ContentType = 0x03,
    ResponseTopic = 0x08,
    CorrelationData = 0x09,
    SubscriptionIdentifier = 0x0B,
    SessionExpiryInterval = 0x11,
    AssignedClientIdentifier = 0x12,
    ServerKeepAlive = 0x13,
    AuthenticationMethod = 0x15,
    AuthenticationData = 0x16,
    RequestProblemInformation = 0x17,
    WillDelayInterval = 0x18,
    RequestResponseInformation = 0x19,
    ResponseInformation = 0x1A,
    ServerReference = 0x1C,
    ReasonString = 0x1F,
    ReceiveMaximum = 0x21,
    TopicAliasMaximum = 0x22,
    TopicAlias = 0x23,
    MaximumQoS = 0x24,
    RetainAvailable = 0x25,
    UserProperty = 0x26,
    MaximumPacketSize = 0x27,
    WildcardSubscriptionAvailable = 0x28,
    SubscriptionIdentifierAvailable = 0x29,
    SharedSubscriptionAvailable = 0x2A,
}

impl PropertyId {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x01 => Some(PropertyId::PayloadFormatIndicator),
            0x02 => Some(PropertyId::MessageExpiryInterval),
            0x03 => Some(PropertyId::ContentType),
            0x08 => Some(PropertyId::ResponseTopic),
            0x09 => Some(PropertyId::CorrelationData),
            0x0B => Some(PropertyId::SubscriptionIdentifier),
            0x11 => Some(PropertyId::SessionExpiryInterval),
            0x12 => Some(PropertyId::AssignedClientIdentifier),
            0x13 => Some(PropertyId::ServerKeepAlive),
            0x15 => Some(PropertyId::AuthenticationMethod),
            0x16 => Some(PropertyId::AuthenticationData),
            0x17 => Some(PropertyId::RequestProblemInformation),
            0x18 => Some(PropertyId::WillDelayInterval),
            0x19 => Some(PropertyId::RequestResponseInformation),
            0x1A => Some(PropertyId::ResponseInformation),
            0x1C => Some(PropertyId::ServerReference),
            0x1F => Some(PropertyId::ReasonString),
            0x21 => Some(PropertyId::ReceiveMaximum),
            0x22 => Some(PropertyId::TopicAliasMaximum),
            0x23 => Some(PropertyId::TopicAlias),
            0x24 => Some(PropertyId::MaximumQoS),
            0x25 => Some(PropertyId::RetainAvailable),
            0x26 => Some(PropertyId::UserProperty),
            0x27 => Some(PropertyId::MaximumPacketSize),
            0x28 => Some(PropertyId::WildcardSubscriptionAvailable),
            0x29 => Some(PropertyId::SubscriptionIdentifierAvailable),
            0x2A => Some(PropertyId::SharedSubscriptionAvailable),
            _ => None,
        }
    }
}

/// A single MQTT property value
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Property {
    PayloadFormatIndicator(u8),
    MessageExpiryInterval(u32),
    ContentType(String),
    ResponseTopic(String),
    CorrelationData(Bytes),
    SubscriptionIdentifier(u32),
    SessionExpiryInterval(u32),
    AssignedClientIdentifier(String),
    ServerKeepAlive(u16),
    AuthenticationMethod(String),
    AuthenticationData(Bytes),
    RequestProblemInformation(u8),
    WillDelayInterval(u32),
    RequestResponseInformation(u8),
    ResponseInformation(String),
    ServerReference(String),
    ReasonString(String),
    ReceiveMaximum(u16),
    TopicAliasMaximum(u16),
    TopicAlias(u16),
    MaximumQoS(u8),
    RetainAvailable(u8),
    UserProperty(String, String),
    MaximumPacketSize(u32),
    WildcardSubscriptionAvailable(u8),
    SubscriptionIdentifierAvailable(u8),
    SharedSubscriptionAvailable(u8),
}

impl Property {
    /// Get the property identifier
    pub fn id(&self) -> PropertyId {
        match self {
            Property::PayloadFormatIndicator(_) => PropertyId::PayloadFormatIndicator,
            Property::MessageExpiryInterval(_) => PropertyId::MessageExpiryInterval,
            Property::ContentType(_) => PropertyId::ContentType,
            Property::ResponseTopic(_) => PropertyId::ResponseTopic,
            Property::CorrelationData(_) => PropertyId::CorrelationData,
            Property::SubscriptionIdentifier(_) => PropertyId::SubscriptionIdentifier,
            Property::SessionExpiryInterval(_) => PropertyId::SessionExpiryInterval,
            Property::AssignedClientIdentifier(_) => PropertyId::AssignedClientIdentifier,
            Property::ServerKeepAlive(_) => PropertyId::ServerKeepAlive,
            Property::AuthenticationMethod(_) => PropertyId::AuthenticationMethod,
            Property::AuthenticationData(_) => PropertyId::AuthenticationData,
            Property::RequestProblemInformation(_) => PropertyId::RequestProblemInformation,
            Property::WillDelayInterval(_) => PropertyId::WillDelayInterval,
            Property::RequestResponseInformation(_) => PropertyId::RequestResponseInformation,
            Property::ResponseInformation(_) => PropertyId::ResponseInformation,
            Property::ServerReference(_) => PropertyId::ServerReference,
            Property::ReasonString(_) => PropertyId::ReasonString,
            Property::ReceiveMaximum(_) => PropertyId::ReceiveMaximum,
            Property::TopicAliasMaximum(_) => PropertyId::TopicAliasMaximum,
            Property::TopicAlias(_) => PropertyId::TopicAlias,
            Property::MaximumQoS(_) => PropertyId::MaximumQoS,
            Property::RetainAvailable(_) => PropertyId::RetainAvailable,
            Property::UserProperty(_, _) => PropertyId::UserProperty,
            Property::MaximumPacketSize(_) => PropertyId::MaximumPacketSize,
            Property::WildcardSubscriptionAvailable(_) => PropertyId::WildcardSubscriptionAvailable,
            Property::SubscriptionIdentifierAvailable(_) => {
                PropertyId::SubscriptionIdentifierAvailable
            }
            Property::SharedSubscriptionAvailable(_) => PropertyId::SharedSubscriptionAvailable,
        }
    }

    /// Calculate encoded size
    pub fn encoded_size(&self) -> usize {
        1 + match self {
            Property::PayloadFormatIndicator(_) => 1,
            Property::MessageExpiryInterval(_) => 4,
            Property::ContentType(s) => 2 + s.len(),
            Property::ResponseTopic(s) => 2 + s.len(),
            Property::CorrelationData(d) => 2 + d.len(),
            Property::SubscriptionIdentifier(v) => variable_int_len(*v),
            Property::SessionExpiryInterval(_) => 4,
            Property::AssignedClientIdentifier(s) => 2 + s.len(),
            Property::ServerKeepAlive(_) => 2,
            Property::AuthenticationMethod(s) => 2 + s.len(),
            Property::AuthenticationData(d) => 2 + d.len(),
            Property::RequestProblemInformation(_) => 1,
            Property::WillDelayInterval(_) => 4,
            Property::RequestResponseInformation(_) => 1,
            Property::ResponseInformation(s) => 2 + s.len(),
            Property::ServerReference(s) => 2 + s.len(),
            Property::ReasonString(s) => 2 + s.len(),
            Property::ReceiveMaximum(_) => 2,
            Property::TopicAliasMaximum(_) => 2,
            Property::TopicAlias(_) => 2,
            Property::MaximumQoS(_) => 1,
            Property::RetainAvailable(_) => 1,
            Property::UserProperty(k, v) => 4 + k.len() + v.len(),
            Property::MaximumPacketSize(_) => 4,
            Property::WildcardSubscriptionAvailable(_) => 1,
            Property::SubscriptionIdentifierAvailable(_) => 1,
            Property::SharedSubscriptionAvailable(_) => 1,
        }
    }
}

/// Collection of MQTT v5.0 properties
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Properties {
    pub payload_format_indicator: Option<u8>,
    pub message_expiry_interval: Option<u32>,
    pub content_type: Option<String>,
    pub response_topic: Option<String>,
    pub correlation_data: Option<Bytes>,
    pub subscription_identifiers: Vec<u32>,
    pub session_expiry_interval: Option<u32>,
    pub assigned_client_identifier: Option<String>,
    pub server_keep_alive: Option<u16>,
    pub authentication_method: Option<String>,
    pub authentication_data: Option<Bytes>,
    pub request_problem_information: Option<u8>,
    pub will_delay_interval: Option<u32>,
    pub request_response_information: Option<u8>,
    pub response_information: Option<String>,
    pub server_reference: Option<String>,
    pub reason_string: Option<String>,
    pub receive_maximum: Option<u16>,
    pub topic_alias_maximum: Option<u16>,
    pub topic_alias: Option<u16>,
    pub maximum_qos: Option<u8>,
    pub retain_available: Option<u8>,
    pub user_properties: Vec<(String, String)>,
    pub maximum_packet_size: Option<u32>,
    pub wildcard_subscription_available: Option<u8>,
    pub subscription_identifier_available: Option<u8>,
    pub shared_subscription_available: Option<u8>,
}

impl Properties {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.payload_format_indicator.is_none()
            && self.message_expiry_interval.is_none()
            && self.content_type.is_none()
            && self.response_topic.is_none()
            && self.correlation_data.is_none()
            && self.subscription_identifiers.is_empty()
            && self.session_expiry_interval.is_none()
            && self.assigned_client_identifier.is_none()
            && self.server_keep_alive.is_none()
            && self.authentication_method.is_none()
            && self.authentication_data.is_none()
            && self.request_problem_information.is_none()
            && self.will_delay_interval.is_none()
            && self.request_response_information.is_none()
            && self.response_information.is_none()
            && self.server_reference.is_none()
            && self.reason_string.is_none()
            && self.receive_maximum.is_none()
            && self.topic_alias_maximum.is_none()
            && self.topic_alias.is_none()
            && self.maximum_qos.is_none()
            && self.retain_available.is_none()
            && self.user_properties.is_empty()
            && self.maximum_packet_size.is_none()
            && self.wildcard_subscription_available.is_none()
            && self.subscription_identifier_available.is_none()
            && self.shared_subscription_available.is_none()
    }

    /// Calculate the encoded size of properties (excluding the length prefix)
    pub fn encoded_size(&self) -> usize {
        let mut size = 0;

        if self.payload_format_indicator.is_some() {
            size += 2; // 1 byte id + 1 byte value
        }
        if self.message_expiry_interval.is_some() {
            size += 5; // 1 byte id + 4 bytes value
        }
        if let Some(ref s) = self.content_type {
            size += 1 + 2 + s.len();
        }
        if let Some(ref s) = self.response_topic {
            size += 1 + 2 + s.len();
        }
        if let Some(ref d) = self.correlation_data {
            size += 1 + 2 + d.len();
        }
        for id in &self.subscription_identifiers {
            size += 1 + variable_int_len(*id);
        }
        if self.session_expiry_interval.is_some() {
            size += 5;
        }
        if let Some(ref s) = self.assigned_client_identifier {
            size += 1 + 2 + s.len();
        }
        if self.server_keep_alive.is_some() {
            size += 3;
        }
        if let Some(ref s) = self.authentication_method {
            size += 1 + 2 + s.len();
        }
        if let Some(ref d) = self.authentication_data {
            size += 1 + 2 + d.len();
        }
        if self.request_problem_information.is_some() {
            size += 2;
        }
        if self.will_delay_interval.is_some() {
            size += 5;
        }
        if self.request_response_information.is_some() {
            size += 2;
        }
        if let Some(ref s) = self.response_information {
            size += 1 + 2 + s.len();
        }
        if let Some(ref s) = self.server_reference {
            size += 1 + 2 + s.len();
        }
        if let Some(ref s) = self.reason_string {
            size += 1 + 2 + s.len();
        }
        if self.receive_maximum.is_some() {
            size += 3;
        }
        if self.topic_alias_maximum.is_some() {
            size += 3;
        }
        if self.topic_alias.is_some() {
            size += 3;
        }
        if self.maximum_qos.is_some() {
            size += 2;
        }
        if self.retain_available.is_some() {
            size += 2;
        }
        for (k, v) in &self.user_properties {
            size += 1 + 2 + k.len() + 2 + v.len();
        }
        if self.maximum_packet_size.is_some() {
            size += 5;
        }
        if self.wildcard_subscription_available.is_some() {
            size += 2;
        }
        if self.subscription_identifier_available.is_some() {
            size += 2;
        }
        if self.shared_subscription_available.is_some() {
            size += 2;
        }

        size
    }

    /// Decode properties from buffer
    pub fn decode(buf: &[u8]) -> Result<(Self, usize), DecodeError> {
        if buf.is_empty() {
            return Err(DecodeError::InsufficientData);
        }

        // Read property length
        let (prop_len, len_bytes) = read_variable_int(buf)?;

        if buf.len() < len_bytes + prop_len as usize {
            return Err(DecodeError::InsufficientData);
        }

        let mut props = Properties::new();
        let mut pos = len_bytes;
        let end = len_bytes + prop_len as usize;

        while pos < end {
            let (prop_id, id_len) = read_variable_int(&buf[pos..])?;
            pos += id_len;

            let prop_id = PropertyId::from_u8(prop_id as u8)
                .ok_or(DecodeError::InvalidPropertyId(prop_id as u8))?;

            match prop_id {
                PropertyId::PayloadFormatIndicator => {
                    if props.payload_format_indicator.is_some() {
                        return Err(DecodeError::DuplicateProperty(prop_id as u8));
                    }
                    if pos >= end {
                        return Err(DecodeError::InsufficientData);
                    }
                    props.payload_format_indicator = Some(buf[pos]);
                    pos += 1;
                }
                PropertyId::MessageExpiryInterval => {
                    if props.message_expiry_interval.is_some() {
                        return Err(DecodeError::DuplicateProperty(prop_id as u8));
                    }
                    if pos + 4 > end {
                        return Err(DecodeError::InsufficientData);
                    }
                    props.message_expiry_interval = Some(u32::from_be_bytes([
                        buf[pos],
                        buf[pos + 1],
                        buf[pos + 2],
                        buf[pos + 3],
                    ]));
                    pos += 4;
                }
                PropertyId::ContentType => {
                    if props.content_type.is_some() {
                        return Err(DecodeError::DuplicateProperty(prop_id as u8));
                    }
                    let (s, len) = read_string(&buf[pos..])?;
                    props.content_type = Some(s.into());
                    pos += len;
                }
                PropertyId::ResponseTopic => {
                    if props.response_topic.is_some() {
                        return Err(DecodeError::DuplicateProperty(prop_id as u8));
                    }
                    let (s, len) = read_string(&buf[pos..])?;
                    props.response_topic = Some(s.into());
                    pos += len;
                }
                PropertyId::CorrelationData => {
                    if props.correlation_data.is_some() {
                        return Err(DecodeError::DuplicateProperty(prop_id as u8));
                    }
                    let (data, len) = read_binary(&buf[pos..])?;
                    props.correlation_data = Some(Bytes::copy_from_slice(data));
                    pos += len;
                }
                PropertyId::SubscriptionIdentifier => {
                    let (val, len) = read_variable_int(&buf[pos..])?;
                    if val == 0 {
                        return Err(DecodeError::MalformedPacket(
                            "subscription identifier cannot be 0",
                        ));
                    }
                    props.subscription_identifiers.push(val);
                    pos += len;
                }
                PropertyId::SessionExpiryInterval => {
                    if props.session_expiry_interval.is_some() {
                        return Err(DecodeError::DuplicateProperty(prop_id as u8));
                    }
                    if pos + 4 > end {
                        return Err(DecodeError::InsufficientData);
                    }
                    props.session_expiry_interval = Some(u32::from_be_bytes([
                        buf[pos],
                        buf[pos + 1],
                        buf[pos + 2],
                        buf[pos + 3],
                    ]));
                    pos += 4;
                }
                PropertyId::AssignedClientIdentifier => {
                    if props.assigned_client_identifier.is_some() {
                        return Err(DecodeError::DuplicateProperty(prop_id as u8));
                    }
                    let (s, len) = read_string(&buf[pos..])?;
                    props.assigned_client_identifier = Some(s.into());
                    pos += len;
                }
                PropertyId::ServerKeepAlive => {
                    if props.server_keep_alive.is_some() {
                        return Err(DecodeError::DuplicateProperty(prop_id as u8));
                    }
                    if pos + 2 > end {
                        return Err(DecodeError::InsufficientData);
                    }
                    props.server_keep_alive = Some(u16::from_be_bytes([buf[pos], buf[pos + 1]]));
                    pos += 2;
                }
                PropertyId::AuthenticationMethod => {
                    if props.authentication_method.is_some() {
                        return Err(DecodeError::DuplicateProperty(prop_id as u8));
                    }
                    let (s, len) = read_string(&buf[pos..])?;
                    props.authentication_method = Some(s.into());
                    pos += len;
                }
                PropertyId::AuthenticationData => {
                    if props.authentication_data.is_some() {
                        return Err(DecodeError::DuplicateProperty(prop_id as u8));
                    }
                    let (data, len) = read_binary(&buf[pos..])?;
                    props.authentication_data = Some(Bytes::copy_from_slice(data));
                    pos += len;
                }
                PropertyId::RequestProblemInformation => {
                    if props.request_problem_information.is_some() {
                        return Err(DecodeError::DuplicateProperty(prop_id as u8));
                    }
                    if pos >= end {
                        return Err(DecodeError::InsufficientData);
                    }
                    props.request_problem_information = Some(buf[pos]);
                    pos += 1;
                }
                PropertyId::WillDelayInterval => {
                    if props.will_delay_interval.is_some() {
                        return Err(DecodeError::DuplicateProperty(prop_id as u8));
                    }
                    if pos + 4 > end {
                        return Err(DecodeError::InsufficientData);
                    }
                    props.will_delay_interval = Some(u32::from_be_bytes([
                        buf[pos],
                        buf[pos + 1],
                        buf[pos + 2],
                        buf[pos + 3],
                    ]));
                    pos += 4;
                }
                PropertyId::RequestResponseInformation => {
                    if props.request_response_information.is_some() {
                        return Err(DecodeError::DuplicateProperty(prop_id as u8));
                    }
                    if pos >= end {
                        return Err(DecodeError::InsufficientData);
                    }
                    props.request_response_information = Some(buf[pos]);
                    pos += 1;
                }
                PropertyId::ResponseInformation => {
                    if props.response_information.is_some() {
                        return Err(DecodeError::DuplicateProperty(prop_id as u8));
                    }
                    let (s, len) = read_string(&buf[pos..])?;
                    props.response_information = Some(s.into());
                    pos += len;
                }
                PropertyId::ServerReference => {
                    if props.server_reference.is_some() {
                        return Err(DecodeError::DuplicateProperty(prop_id as u8));
                    }
                    let (s, len) = read_string(&buf[pos..])?;
                    props.server_reference = Some(s.into());
                    pos += len;
                }
                PropertyId::ReasonString => {
                    if props.reason_string.is_some() {
                        return Err(DecodeError::DuplicateProperty(prop_id as u8));
                    }
                    let (s, len) = read_string(&buf[pos..])?;
                    props.reason_string = Some(s.into());
                    pos += len;
                }
                PropertyId::ReceiveMaximum => {
                    if props.receive_maximum.is_some() {
                        return Err(DecodeError::DuplicateProperty(prop_id as u8));
                    }
                    if pos + 2 > end {
                        return Err(DecodeError::InsufficientData);
                    }
                    let val = u16::from_be_bytes([buf[pos], buf[pos + 1]]);
                    if val == 0 {
                        return Err(DecodeError::MalformedPacket("receive maximum cannot be 0"));
                    }
                    props.receive_maximum = Some(val);
                    pos += 2;
                }
                PropertyId::TopicAliasMaximum => {
                    if props.topic_alias_maximum.is_some() {
                        return Err(DecodeError::DuplicateProperty(prop_id as u8));
                    }
                    if pos + 2 > end {
                        return Err(DecodeError::InsufficientData);
                    }
                    props.topic_alias_maximum = Some(u16::from_be_bytes([buf[pos], buf[pos + 1]]));
                    pos += 2;
                }
                PropertyId::TopicAlias => {
                    if props.topic_alias.is_some() {
                        return Err(DecodeError::DuplicateProperty(prop_id as u8));
                    }
                    if pos + 2 > end {
                        return Err(DecodeError::InsufficientData);
                    }
                    let val = u16::from_be_bytes([buf[pos], buf[pos + 1]]);
                    if val == 0 {
                        return Err(DecodeError::MalformedPacket("topic alias cannot be 0"));
                    }
                    props.topic_alias = Some(val);
                    pos += 2;
                }
                PropertyId::MaximumQoS => {
                    if props.maximum_qos.is_some() {
                        return Err(DecodeError::DuplicateProperty(prop_id as u8));
                    }
                    if pos >= end {
                        return Err(DecodeError::InsufficientData);
                    }
                    props.maximum_qos = Some(buf[pos]);
                    pos += 1;
                }
                PropertyId::RetainAvailable => {
                    if props.retain_available.is_some() {
                        return Err(DecodeError::DuplicateProperty(prop_id as u8));
                    }
                    if pos >= end {
                        return Err(DecodeError::InsufficientData);
                    }
                    props.retain_available = Some(buf[pos]);
                    pos += 1;
                }
                PropertyId::UserProperty => {
                    let (key, key_len) = read_string(&buf[pos..])?;
                    pos += key_len;
                    let (val, val_len) = read_string(&buf[pos..])?;
                    pos += val_len;
                    props
                        .user_properties
                        .push((key.to_string(), val.to_string()));
                }
                PropertyId::MaximumPacketSize => {
                    if props.maximum_packet_size.is_some() {
                        return Err(DecodeError::DuplicateProperty(prop_id as u8));
                    }
                    if pos + 4 > end {
                        return Err(DecodeError::InsufficientData);
                    }
                    let val =
                        u32::from_be_bytes([buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]]);
                    if val == 0 {
                        return Err(DecodeError::MalformedPacket(
                            "maximum packet size cannot be 0",
                        ));
                    }
                    props.maximum_packet_size = Some(val);
                    pos += 4;
                }
                PropertyId::WildcardSubscriptionAvailable => {
                    if props.wildcard_subscription_available.is_some() {
                        return Err(DecodeError::DuplicateProperty(prop_id as u8));
                    }
                    if pos >= end {
                        return Err(DecodeError::InsufficientData);
                    }
                    props.wildcard_subscription_available = Some(buf[pos]);
                    pos += 1;
                }
                PropertyId::SubscriptionIdentifierAvailable => {
                    if props.subscription_identifier_available.is_some() {
                        return Err(DecodeError::DuplicateProperty(prop_id as u8));
                    }
                    if pos >= end {
                        return Err(DecodeError::InsufficientData);
                    }
                    props.subscription_identifier_available = Some(buf[pos]);
                    pos += 1;
                }
                PropertyId::SharedSubscriptionAvailable => {
                    if props.shared_subscription_available.is_some() {
                        return Err(DecodeError::DuplicateProperty(prop_id as u8));
                    }
                    if pos >= end {
                        return Err(DecodeError::InsufficientData);
                    }
                    props.shared_subscription_available = Some(buf[pos]);
                    pos += 1;
                }
            }
        }

        Ok((props, end))
    }

    /// Encode properties to buffer
    pub fn encode(&self, buf: &mut BytesMut) -> Result<(), EncodeError> {
        let size = self.encoded_size();
        write_variable_int(buf, size as u32)?;

        if let Some(v) = self.payload_format_indicator {
            buf.put_u8(PropertyId::PayloadFormatIndicator as u8);
            buf.put_u8(v);
        }
        if let Some(v) = self.message_expiry_interval {
            buf.put_u8(PropertyId::MessageExpiryInterval as u8);
            buf.put_u32(v);
        }
        if let Some(ref s) = self.content_type {
            buf.put_u8(PropertyId::ContentType as u8);
            write_string(buf, s)?;
        }
        if let Some(ref s) = self.response_topic {
            buf.put_u8(PropertyId::ResponseTopic as u8);
            write_string(buf, s)?;
        }
        if let Some(ref d) = self.correlation_data {
            buf.put_u8(PropertyId::CorrelationData as u8);
            write_binary(buf, d)?;
        }
        for id in &self.subscription_identifiers {
            buf.put_u8(PropertyId::SubscriptionIdentifier as u8);
            write_variable_int(buf, *id)?;
        }
        if let Some(v) = self.session_expiry_interval {
            buf.put_u8(PropertyId::SessionExpiryInterval as u8);
            buf.put_u32(v);
        }
        if let Some(ref s) = self.assigned_client_identifier {
            buf.put_u8(PropertyId::AssignedClientIdentifier as u8);
            write_string(buf, s)?;
        }
        if let Some(v) = self.server_keep_alive {
            buf.put_u8(PropertyId::ServerKeepAlive as u8);
            buf.put_u16(v);
        }
        if let Some(ref s) = self.authentication_method {
            buf.put_u8(PropertyId::AuthenticationMethod as u8);
            write_string(buf, s)?;
        }
        if let Some(ref d) = self.authentication_data {
            buf.put_u8(PropertyId::AuthenticationData as u8);
            write_binary(buf, d)?;
        }
        if let Some(v) = self.request_problem_information {
            buf.put_u8(PropertyId::RequestProblemInformation as u8);
            buf.put_u8(v);
        }
        if let Some(v) = self.will_delay_interval {
            buf.put_u8(PropertyId::WillDelayInterval as u8);
            buf.put_u32(v);
        }
        if let Some(v) = self.request_response_information {
            buf.put_u8(PropertyId::RequestResponseInformation as u8);
            buf.put_u8(v);
        }
        if let Some(ref s) = self.response_information {
            buf.put_u8(PropertyId::ResponseInformation as u8);
            write_string(buf, s)?;
        }
        if let Some(ref s) = self.server_reference {
            buf.put_u8(PropertyId::ServerReference as u8);
            write_string(buf, s)?;
        }
        if let Some(ref s) = self.reason_string {
            buf.put_u8(PropertyId::ReasonString as u8);
            write_string(buf, s)?;
        }
        if let Some(v) = self.receive_maximum {
            buf.put_u8(PropertyId::ReceiveMaximum as u8);
            buf.put_u16(v);
        }
        if let Some(v) = self.topic_alias_maximum {
            buf.put_u8(PropertyId::TopicAliasMaximum as u8);
            buf.put_u16(v);
        }
        if let Some(v) = self.topic_alias {
            buf.put_u8(PropertyId::TopicAlias as u8);
            buf.put_u16(v);
        }
        if let Some(v) = self.maximum_qos {
            buf.put_u8(PropertyId::MaximumQoS as u8);
            buf.put_u8(v);
        }
        if let Some(v) = self.retain_available {
            buf.put_u8(PropertyId::RetainAvailable as u8);
            buf.put_u8(v);
        }
        for (k, v) in &self.user_properties {
            buf.put_u8(PropertyId::UserProperty as u8);
            write_string(buf, k)?;
            write_string(buf, v)?;
        }
        if let Some(v) = self.maximum_packet_size {
            buf.put_u8(PropertyId::MaximumPacketSize as u8);
            buf.put_u32(v);
        }
        if let Some(v) = self.wildcard_subscription_available {
            buf.put_u8(PropertyId::WildcardSubscriptionAvailable as u8);
            buf.put_u8(v);
        }
        if let Some(v) = self.subscription_identifier_available {
            buf.put_u8(PropertyId::SubscriptionIdentifierAvailable as u8);
            buf.put_u8(v);
        }
        if let Some(v) = self.shared_subscription_available {
            buf.put_u8(PropertyId::SharedSubscriptionAvailable as u8);
            buf.put_u8(v);
        }

        Ok(())
    }
}

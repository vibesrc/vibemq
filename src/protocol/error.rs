//! Protocol error types

use std::fmt;

/// Errors that can occur during packet decoding
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// Not enough data in buffer
    InsufficientData,
    /// Invalid packet type
    InvalidPacketType(u8),
    /// Invalid remaining length encoding
    InvalidRemainingLength,
    /// Remaining length exceeds maximum
    RemainingLengthTooLarge,
    /// Invalid protocol name
    InvalidProtocolName,
    /// Invalid protocol version
    InvalidProtocolVersion(u8),
    /// Invalid QoS value
    InvalidQoS(u8),
    /// Invalid UTF-8 string
    InvalidUtf8,
    /// String exceeds maximum length
    StringTooLong,
    /// Invalid property identifier
    InvalidPropertyId(u8),
    /// Duplicate property (not allowed)
    DuplicateProperty(u8),
    /// Invalid packet flags
    InvalidFlags,
    /// Malformed packet
    MalformedPacket(&'static str),
    /// Packet too large
    PacketTooLarge,
    /// Invalid reason code
    InvalidReasonCode(u8),
    /// Invalid subscription options
    InvalidSubscriptionOptions,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InsufficientData => write!(f, "insufficient data in buffer"),
            Self::InvalidPacketType(t) => write!(f, "invalid packet type: {}", t),
            Self::InvalidRemainingLength => write!(f, "invalid remaining length encoding"),
            Self::RemainingLengthTooLarge => write!(f, "remaining length exceeds maximum"),
            Self::InvalidProtocolName => write!(f, "invalid protocol name"),
            Self::InvalidProtocolVersion(v) => write!(f, "invalid protocol version: {}", v),
            Self::InvalidQoS(q) => write!(f, "invalid QoS value: {}", q),
            Self::InvalidUtf8 => write!(f, "invalid UTF-8 string"),
            Self::StringTooLong => write!(f, "string exceeds maximum length"),
            Self::InvalidPropertyId(id) => write!(f, "invalid property identifier: {}", id),
            Self::DuplicateProperty(id) => write!(f, "duplicate property: {}", id),
            Self::InvalidFlags => write!(f, "invalid packet flags"),
            Self::MalformedPacket(msg) => write!(f, "malformed packet: {}", msg),
            Self::PacketTooLarge => write!(f, "packet too large"),
            Self::InvalidReasonCode(r) => write!(f, "invalid reason code: {}", r),
            Self::InvalidSubscriptionOptions => write!(f, "invalid subscription options"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Errors that can occur during packet encoding
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodeError {
    /// Buffer too small
    BufferTooSmall,
    /// Packet too large
    PacketTooLarge,
    /// String too long
    StringTooLong,
    /// Invalid topic name
    InvalidTopicName,
    /// Too many subscriptions
    TooManySubscriptions,
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BufferTooSmall => write!(f, "buffer too small"),
            Self::PacketTooLarge => write!(f, "packet too large"),
            Self::StringTooLong => write!(f, "string too long"),
            Self::InvalidTopicName => write!(f, "invalid topic name"),
            Self::TooManySubscriptions => write!(f, "too many subscriptions"),
        }
    }
}

impl std::error::Error for EncodeError {}

/// High-level protocol errors
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    /// Decode error
    Decode(DecodeError),
    /// Encode error
    Encode(EncodeError),
    /// Connection refused
    ConnectionRefused(u8),
    /// Protocol violation
    ProtocolViolation(&'static str),
    /// Session expired
    SessionExpired,
    /// Not authorized
    NotAuthorized,
    /// Quota exceeded
    QuotaExceeded,
    /// Keep alive timeout
    KeepAliveTimeout,
    /// Implementation specific error
    ImplementationError(&'static str),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(e) => write!(f, "decode error: {}", e),
            Self::Encode(e) => write!(f, "encode error: {}", e),
            Self::ConnectionRefused(code) => write!(f, "connection refused: {}", code),
            Self::ProtocolViolation(msg) => write!(f, "protocol violation: {}", msg),
            Self::SessionExpired => write!(f, "session expired"),
            Self::NotAuthorized => write!(f, "not authorized"),
            Self::QuotaExceeded => write!(f, "quota exceeded"),
            Self::KeepAliveTimeout => write!(f, "keep alive timeout"),
            Self::ImplementationError(msg) => write!(f, "implementation error: {}", msg),
        }
    }
}

impl std::error::Error for ProtocolError {}

impl From<DecodeError> for ProtocolError {
    fn from(e: DecodeError) -> Self {
        ProtocolError::Decode(e)
    }
}

impl From<EncodeError> for ProtocolError {
    fn from(e: EncodeError) -> Self {
        ProtocolError::Encode(e)
    }
}

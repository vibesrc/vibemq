//! MQTT v5.0 Reason Codes
//!
//! Based on spec/v5.0/2.4_reason-code.md

use std::fmt;

/// MQTT v5.0 Reason Code
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
#[derive(Default)]
pub enum ReasonCode {
    /// Success / Normal disconnection / Granted QoS 0
    #[default]
    Success = 0x00,
    /// Granted QoS 1
    GrantedQoS1 = 0x01,
    /// Granted QoS 2
    GrantedQoS2 = 0x02,
    /// Disconnect with Will Message
    DisconnectWithWill = 0x04,
    /// No matching subscribers
    NoMatchingSubscribers = 0x10,
    /// No subscription existed
    NoSubscriptionExisted = 0x11,
    /// Continue authentication
    ContinueAuthentication = 0x18,
    /// Re-authenticate
    ReAuthenticate = 0x19,
    /// Unspecified error
    UnspecifiedError = 0x80,
    /// Malformed Packet
    MalformedPacket = 0x81,
    /// Protocol Error
    ProtocolError = 0x82,
    /// Implementation specific error
    ImplementationError = 0x83,
    /// Unsupported Protocol Version
    UnsupportedProtocolVersion = 0x84,
    /// Client Identifier not valid
    ClientIdNotValid = 0x85,
    /// Bad User Name or Password
    BadUserNameOrPassword = 0x86,
    /// Not authorized
    NotAuthorized = 0x87,
    /// Server unavailable
    ServerUnavailable = 0x88,
    /// Server busy
    ServerBusy = 0x89,
    /// Banned
    Banned = 0x8A,
    /// Server shutting down
    ServerShuttingDown = 0x8B,
    /// Bad authentication method
    BadAuthenticationMethod = 0x8C,
    /// Keep Alive timeout
    KeepAliveTimeout = 0x8D,
    /// Session taken over
    SessionTakenOver = 0x8E,
    /// Topic Filter invalid
    TopicFilterInvalid = 0x8F,
    /// Topic Name invalid
    TopicNameInvalid = 0x90,
    /// Packet Identifier in use
    PacketIdInUse = 0x91,
    /// Packet Identifier not found
    PacketIdNotFound = 0x92,
    /// Receive Maximum exceeded
    ReceiveMaxExceeded = 0x93,
    /// Topic Alias invalid
    TopicAliasInvalid = 0x94,
    /// Packet too large
    PacketTooLarge = 0x95,
    /// Message rate too high
    MessageRateTooHigh = 0x96,
    /// Quota exceeded
    QuotaExceeded = 0x97,
    /// Administrative action
    AdministrativeAction = 0x98,
    /// Payload format invalid
    PayloadFormatInvalid = 0x99,
    /// Retain not supported
    RetainNotSupported = 0x9A,
    /// QoS not supported
    QoSNotSupported = 0x9B,
    /// Use another server
    UseAnotherServer = 0x9C,
    /// Server moved
    ServerMoved = 0x9D,
    /// Shared Subscriptions not supported
    SharedSubsNotSupported = 0x9E,
    /// Connection rate exceeded
    ConnectionRateExceeded = 0x9F,
    /// Maximum connect time
    MaximumConnectTime = 0xA0,
    /// Subscription Identifiers not supported
    SubIdNotSupported = 0xA1,
    /// Wildcard Subscriptions not supported
    WildcardSubsNotSupported = 0xA2,
}

impl ReasonCode {
    /// Create a ReasonCode from a byte value
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x00 => Some(ReasonCode::Success),
            0x01 => Some(ReasonCode::GrantedQoS1),
            0x02 => Some(ReasonCode::GrantedQoS2),
            0x04 => Some(ReasonCode::DisconnectWithWill),
            0x10 => Some(ReasonCode::NoMatchingSubscribers),
            0x11 => Some(ReasonCode::NoSubscriptionExisted),
            0x18 => Some(ReasonCode::ContinueAuthentication),
            0x19 => Some(ReasonCode::ReAuthenticate),
            0x80 => Some(ReasonCode::UnspecifiedError),
            0x81 => Some(ReasonCode::MalformedPacket),
            0x82 => Some(ReasonCode::ProtocolError),
            0x83 => Some(ReasonCode::ImplementationError),
            0x84 => Some(ReasonCode::UnsupportedProtocolVersion),
            0x85 => Some(ReasonCode::ClientIdNotValid),
            0x86 => Some(ReasonCode::BadUserNameOrPassword),
            0x87 => Some(ReasonCode::NotAuthorized),
            0x88 => Some(ReasonCode::ServerUnavailable),
            0x89 => Some(ReasonCode::ServerBusy),
            0x8A => Some(ReasonCode::Banned),
            0x8B => Some(ReasonCode::ServerShuttingDown),
            0x8C => Some(ReasonCode::BadAuthenticationMethod),
            0x8D => Some(ReasonCode::KeepAliveTimeout),
            0x8E => Some(ReasonCode::SessionTakenOver),
            0x8F => Some(ReasonCode::TopicFilterInvalid),
            0x90 => Some(ReasonCode::TopicNameInvalid),
            0x91 => Some(ReasonCode::PacketIdInUse),
            0x92 => Some(ReasonCode::PacketIdNotFound),
            0x93 => Some(ReasonCode::ReceiveMaxExceeded),
            0x94 => Some(ReasonCode::TopicAliasInvalid),
            0x95 => Some(ReasonCode::PacketTooLarge),
            0x96 => Some(ReasonCode::MessageRateTooHigh),
            0x97 => Some(ReasonCode::QuotaExceeded),
            0x98 => Some(ReasonCode::AdministrativeAction),
            0x99 => Some(ReasonCode::PayloadFormatInvalid),
            0x9A => Some(ReasonCode::RetainNotSupported),
            0x9B => Some(ReasonCode::QoSNotSupported),
            0x9C => Some(ReasonCode::UseAnotherServer),
            0x9D => Some(ReasonCode::ServerMoved),
            0x9E => Some(ReasonCode::SharedSubsNotSupported),
            0x9F => Some(ReasonCode::ConnectionRateExceeded),
            0xA0 => Some(ReasonCode::MaximumConnectTime),
            0xA1 => Some(ReasonCode::SubIdNotSupported),
            0xA2 => Some(ReasonCode::WildcardSubsNotSupported),
            _ => None,
        }
    }

    /// Check if this reason code indicates success
    #[inline]
    pub fn is_success(self) -> bool {
        (self as u8) < 0x80
    }

    /// Check if this reason code indicates failure
    #[inline]
    pub fn is_error(self) -> bool {
        (self as u8) >= 0x80
    }

    /// Convert to MQTT v3.1.1 CONNACK return code if applicable
    pub fn to_v3_connack_code(self) -> u8 {
        match self {
            ReasonCode::Success => 0x00,
            ReasonCode::UnsupportedProtocolVersion => 0x01,
            ReasonCode::ClientIdNotValid => 0x02,
            ReasonCode::ServerUnavailable => 0x03,
            ReasonCode::BadUserNameOrPassword => 0x04,
            ReasonCode::NotAuthorized => 0x05,
            _ => 0x05, // Default to "not authorized" for other errors
        }
    }

    /// Create from MQTT v3.1.1 CONNACK return code
    pub fn from_v3_connack_code(code: u8) -> Self {
        match code {
            0x00 => ReasonCode::Success,
            0x01 => ReasonCode::UnsupportedProtocolVersion,
            0x02 => ReasonCode::ClientIdNotValid,
            0x03 => ReasonCode::ServerUnavailable,
            0x04 => ReasonCode::BadUserNameOrPassword,
            0x05 => ReasonCode::NotAuthorized,
            _ => ReasonCode::UnspecifiedError,
        }
    }
}

impl fmt::Display for ReasonCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReasonCode::Success => write!(f, "Success"),
            ReasonCode::GrantedQoS1 => write!(f, "Granted QoS 1"),
            ReasonCode::GrantedQoS2 => write!(f, "Granted QoS 2"),
            ReasonCode::DisconnectWithWill => write!(f, "Disconnect with Will Message"),
            ReasonCode::NoMatchingSubscribers => write!(f, "No matching subscribers"),
            ReasonCode::NoSubscriptionExisted => write!(f, "No subscription existed"),
            ReasonCode::ContinueAuthentication => write!(f, "Continue authentication"),
            ReasonCode::ReAuthenticate => write!(f, "Re-authenticate"),
            ReasonCode::UnspecifiedError => write!(f, "Unspecified error"),
            ReasonCode::MalformedPacket => write!(f, "Malformed Packet"),
            ReasonCode::ProtocolError => write!(f, "Protocol Error"),
            ReasonCode::ImplementationError => write!(f, "Implementation specific error"),
            ReasonCode::UnsupportedProtocolVersion => write!(f, "Unsupported Protocol Version"),
            ReasonCode::ClientIdNotValid => write!(f, "Client Identifier not valid"),
            ReasonCode::BadUserNameOrPassword => write!(f, "Bad User Name or Password"),
            ReasonCode::NotAuthorized => write!(f, "Not authorized"),
            ReasonCode::ServerUnavailable => write!(f, "Server unavailable"),
            ReasonCode::ServerBusy => write!(f, "Server busy"),
            ReasonCode::Banned => write!(f, "Banned"),
            ReasonCode::ServerShuttingDown => write!(f, "Server shutting down"),
            ReasonCode::BadAuthenticationMethod => write!(f, "Bad authentication method"),
            ReasonCode::KeepAliveTimeout => write!(f, "Keep Alive timeout"),
            ReasonCode::SessionTakenOver => write!(f, "Session taken over"),
            ReasonCode::TopicFilterInvalid => write!(f, "Topic Filter invalid"),
            ReasonCode::TopicNameInvalid => write!(f, "Topic Name invalid"),
            ReasonCode::PacketIdInUse => write!(f, "Packet Identifier in use"),
            ReasonCode::PacketIdNotFound => write!(f, "Packet Identifier not found"),
            ReasonCode::ReceiveMaxExceeded => write!(f, "Receive Maximum exceeded"),
            ReasonCode::TopicAliasInvalid => write!(f, "Topic Alias invalid"),
            ReasonCode::PacketTooLarge => write!(f, "Packet too large"),
            ReasonCode::MessageRateTooHigh => write!(f, "Message rate too high"),
            ReasonCode::QuotaExceeded => write!(f, "Quota exceeded"),
            ReasonCode::AdministrativeAction => write!(f, "Administrative action"),
            ReasonCode::PayloadFormatInvalid => write!(f, "Payload format invalid"),
            ReasonCode::RetainNotSupported => write!(f, "Retain not supported"),
            ReasonCode::QoSNotSupported => write!(f, "QoS not supported"),
            ReasonCode::UseAnotherServer => write!(f, "Use another server"),
            ReasonCode::ServerMoved => write!(f, "Server moved"),
            ReasonCode::SharedSubsNotSupported => write!(f, "Shared Subscriptions not supported"),
            ReasonCode::ConnectionRateExceeded => write!(f, "Connection rate exceeded"),
            ReasonCode::MaximumConnectTime => write!(f, "Maximum connect time"),
            ReasonCode::SubIdNotSupported => write!(f, "Subscription Identifiers not supported"),
            ReasonCode::WildcardSubsNotSupported => {
                write!(f, "Wildcard Subscriptions not supported")
            }
        }
    }
}

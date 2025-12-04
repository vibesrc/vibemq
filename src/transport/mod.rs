//! Transport Layer
//!
//! Handles TCP and WebSocket connections with a unified interface.

mod websocket;

pub use websocket::WsStream;

use tokio::net::TcpStream;

/// Transport configuration
#[derive(Debug, Clone)]
pub struct TransportConfig {
    /// TCP nodelay
    pub tcp_nodelay: bool,
    /// TCP keepalive
    pub tcp_keepalive: Option<std::time::Duration>,
    /// Socket receive buffer size
    pub recv_buffer_size: Option<usize>,
    /// Socket send buffer size
    pub send_buffer_size: Option<usize>,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            tcp_nodelay: true,
            tcp_keepalive: Some(std::time::Duration::from_secs(60)),
            recv_buffer_size: None,
            send_buffer_size: None,
        }
    }
}

/// Configure a TCP stream
pub fn configure_stream(stream: &TcpStream, config: &TransportConfig) -> std::io::Result<()> {
    stream.set_nodelay(config.tcp_nodelay)?;

    // Note: TCP keepalive configuration requires platform-specific code
    // which is not implemented here for simplicity

    Ok(())
}

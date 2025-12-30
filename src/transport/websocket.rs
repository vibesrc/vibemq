//! WebSocket Transport
//!
//! Provides a wrapper around tokio-tungstenite WebSocket that implements
//! AsyncRead and AsyncWrite for use with MQTT over WebSocket.

use std::collections::VecDeque;
use std::io::{self};
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::BytesMut;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{Sink, Stream, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::WebSocketStream;

/// WebSocket stream wrapper that implements AsyncRead and AsyncWrite
///
/// MQTT over WebSocket uses binary frames to transport MQTT packets.
/// This wrapper buffers incoming binary messages and presents them
/// as a continuous byte stream.
pub struct WsStream {
    /// Split sink for writing
    sink: SplitSink<WebSocketStream<TcpStream>, Message>,
    /// Split stream for reading
    stream: SplitStream<WebSocketStream<TcpStream>>,
    /// Read buffer for incomplete reads
    read_buffer: BytesMut,
    /// Write buffer for batching small writes
    write_buffer: BytesMut,
    /// Pending messages to be read
    pending_messages: VecDeque<Vec<u8>>,
    /// Whether the stream has been closed
    closed: bool,
}

impl WsStream {
    /// Create a new WebSocket stream wrapper
    pub fn new(ws: WebSocketStream<TcpStream>) -> Self {
        let (sink, stream) = ws.split();
        Self {
            sink,
            stream,
            read_buffer: BytesMut::with_capacity(2048),
            write_buffer: BytesMut::with_capacity(2048),
            pending_messages: VecDeque::new(),
            closed: false,
        }
    }

    /// Accept a WebSocket connection with MQTT subprotocol
    pub async fn accept(stream: TcpStream) -> Result<Self, io::Error> {
        Self::accept_with_path(stream, "/mqtt").await
    }

    /// Accept a WebSocket connection with MQTT subprotocol and path validation
    pub async fn accept_with_path(
        stream: TcpStream,
        expected_path: &str,
    ) -> Result<Self, io::Error> {
        let expected_path = expected_path.to_string();

        // Custom callback to check for MQTT subprotocol and validate path
        let ws = tokio_tungstenite::accept_hdr_async(stream, move |req: &tokio_tungstenite::tungstenite::handshake::server::Request, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| {
            // Validate request path
            let request_path = req.uri().path();
            if request_path != expected_path {
                return Err(tokio_tungstenite::tungstenite::handshake::server::ErrorResponse::new(Some(format!(
                    "Invalid path: expected '{}', got '{}'",
                    expected_path, request_path
                ))));
            }

            // Check for MQTT subprotocol
            if let Some(protocols) = req.headers().get("sec-websocket-protocol") {
                if let Ok(protocols_str) = protocols.to_str() {
                    for protocol in protocols_str.split(',').map(|s| s.trim()) {
                        if protocol == "mqtt" || protocol == "mqttv3.1" || protocol == "mqttv5" {
                            response.headers_mut().insert(
                                "sec-websocket-protocol",
                                protocol.parse().unwrap(),
                            );
                            break;
                        }
                    }
                }
            }
            Ok(response)
        })
        .await
        .map_err(io::Error::other)?;

        Ok(Self::new(ws))
    }
}

impl AsyncRead for WsStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        // First, try to fulfill from read buffer
        if !self.read_buffer.is_empty() {
            let to_copy = std::cmp::min(buf.remaining(), self.read_buffer.len());
            buf.put_slice(&self.read_buffer[..to_copy]);
            let _ = self.read_buffer.split_to(to_copy);
            return Poll::Ready(Ok(()));
        }

        // Check pending messages
        if let Some(msg) = self.pending_messages.pop_front() {
            let to_copy = std::cmp::min(buf.remaining(), msg.len());
            buf.put_slice(&msg[..to_copy]);
            if to_copy < msg.len() {
                // Store remainder in read buffer
                self.read_buffer.extend_from_slice(&msg[to_copy..]);
            }
            return Poll::Ready(Ok(()));
        }

        if self.closed {
            return Poll::Ready(Ok(()));
        }

        // Poll for new messages
        match Pin::new(&mut self.stream).poll_next(cx) {
            Poll::Ready(Some(Ok(message))) => {
                match message {
                    Message::Binary(data) => {
                        let to_copy = std::cmp::min(buf.remaining(), data.len());
                        buf.put_slice(&data[..to_copy]);
                        if to_copy < data.len() {
                            // Store remainder in read buffer
                            self.read_buffer.extend_from_slice(&data[to_copy..]);
                        }
                        Poll::Ready(Ok(()))
                    }
                    Message::Close(_) => {
                        self.closed = true;
                        Poll::Ready(Ok(()))
                    }
                    Message::Ping(_) | Message::Pong(_) | Message::Text(_) => {
                        // Ignore non-binary messages, try again
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    }
                    Message::Frame(_) => {
                        // Raw frame, ignore
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    }
                }
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Err(io::Error::other(e))),
            Poll::Ready(None) => {
                self.closed = true;
                Poll::Ready(Ok(()))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncWrite for WsStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        // Buffer the write
        self.write_buffer.extend_from_slice(buf);

        // Try to send as binary message
        let data = self.write_buffer.split().freeze().to_vec();
        let message = Message::Binary(data);

        match Pin::new(&mut self.sink).poll_ready(cx) {
            Poll::Ready(Ok(())) => match Pin::new(&mut self.sink).start_send(message) {
                Ok(()) => Poll::Ready(Ok(buf.len())),
                Err(e) => Poll::Ready(Err(io::Error::other(e))),
            },
            Poll::Ready(Err(e)) => Poll::Ready(Err(io::Error::other(e))),
            Poll::Pending => {
                // Put data back in buffer
                // Note: This is a simplification; in practice we'd need more careful handling
                Poll::Pending
            }
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match Pin::new(&mut self.sink).poll_flush(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
            Poll::Ready(Err(e)) => Poll::Ready(Err(io::Error::other(e))),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // Send close message
        match Pin::new(&mut self.sink).poll_ready(cx) {
            Poll::Ready(Ok(())) => {
                let _ = Pin::new(&mut self.sink).start_send(Message::Close(None));
                match Pin::new(&mut self.sink).poll_flush(cx) {
                    Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
                    Poll::Ready(Err(e)) => Poll::Ready(Err(io::Error::other(e))),
                    Poll::Pending => Poll::Pending,
                }
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(io::Error::other(e))),
            Poll::Pending => Poll::Pending,
        }
    }
}

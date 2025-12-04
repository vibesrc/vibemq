//! HTTP server for Prometheus metrics endpoint

use super::Metrics;
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use prometheus::{Encoder, TextEncoder};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{error, info};

/// HTTP server that exposes Prometheus metrics
pub struct MetricsServer {
    metrics: Arc<Metrics>,
    addr: SocketAddr,
}

impl MetricsServer {
    pub fn new(metrics: Arc<Metrics>, addr: SocketAddr) -> Self {
        Self { metrics, addr }
    }

    pub async fn run(self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = TcpListener::bind(self.addr).await?;
        info!("Metrics server listening on http://{}/metrics", self.addr);

        loop {
            let (stream, _) = listener.accept().await?;
            let io = TokioIo::new(stream);
            let metrics = self.metrics.clone();

            tokio::spawn(async move {
                let service = service_fn(move |req| {
                    let metrics = metrics.clone();
                    async move { handle_request(req, metrics).await }
                });

                if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                    error!("Error serving metrics connection: {:?}", err);
                }
            });
        }
    }
}

async fn handle_request(
    req: Request<hyper::body::Incoming>,
    metrics: Arc<Metrics>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let response = match req.uri().path() {
        "/metrics" => {
            let encoder = TextEncoder::new();
            let metric_families = metrics.registry.gather();
            let mut buffer = Vec::new();

            match encoder.encode(&metric_families, &mut buffer) {
                Ok(_) => Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", encoder.format_type())
                    .body(Full::new(Bytes::from(buffer)))
                    .unwrap(),
                Err(e) => {
                    error!("Failed to encode metrics: {}", e);
                    Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Full::new(Bytes::from("Failed to encode metrics")))
                        .unwrap()
                }
            }
        }
        "/health" | "/healthz" => Response::builder()
            .status(StatusCode::OK)
            .body(Full::new(Bytes::from("OK")))
            .unwrap(),
        "/ready" | "/readyz" => Response::builder()
            .status(StatusCode::OK)
            .body(Full::new(Bytes::from("OK")))
            .unwrap(),
        _ => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Full::new(Bytes::from("Not Found")))
            .unwrap(),
    };

    Ok(response)
}

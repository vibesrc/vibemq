//! Prometheus metrics for VibeMQ
//!
//! Exposes metrics at /metrics endpoint for monitoring and observability.
//! Useful for Grafana dashboards, alerts, and capacity planning.

use prometheus::{
    Histogram, HistogramOpts, IntCounter, IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry,
};

mod server;

pub use server::MetricsServer;

/// All VibeMQ metrics in one place
#[derive(Clone)]
pub struct Metrics {
    pub registry: Registry,

    // Connection metrics
    pub connections_total: IntCounter,
    pub connections_current: IntGauge,
    pub connections_by_protocol: IntGaugeVec,

    // Message metrics
    pub messages_received_total: IntCounterVec,
    pub messages_sent_total: IntCounterVec,
    pub messages_bytes_received: IntCounter,
    pub messages_bytes_sent: IntCounter,

    // Subscription metrics
    pub subscriptions_current: IntGauge,
    pub subscriptions_total: IntCounter,
    pub unsubscriptions_total: IntCounter,

    // Retained messages
    pub retained_messages_current: IntGauge,
    pub retained_bytes_current: IntGauge,

    // QoS metrics
    pub inflight_messages: IntGaugeVec,
    pub qos1_retransmits: IntCounter,
    pub qos2_retransmits: IntCounter,

    // Cluster metrics
    pub cluster_peers_current: IntGauge,
    pub cluster_messages_forwarded: IntCounter,
    pub cluster_messages_received: IntCounter,

    // Performance metrics
    pub publish_latency: Histogram,
    pub connect_duration: Histogram,
}

impl Metrics {
    pub fn new() -> Self {
        let registry = Registry::new();

        // Connection metrics
        let connections_total = IntCounter::with_opts(Opts::new(
            "vibemq_connections_total",
            "Total number of client connections since startup",
        ))
        .unwrap();

        let connections_current = IntGauge::with_opts(Opts::new(
            "vibemq_connections_current",
            "Current number of connected clients",
        ))
        .unwrap();

        let connections_by_protocol = IntGaugeVec::new(
            Opts::new(
                "vibemq_connections_by_protocol",
                "Current connections by protocol version",
            ),
            &["protocol"],
        )
        .unwrap();

        // Message metrics
        let messages_received_total = IntCounterVec::new(
            Opts::new(
                "vibemq_messages_received_total",
                "Total messages received by type",
            ),
            &["type"],
        )
        .unwrap();

        let messages_sent_total = IntCounterVec::new(
            Opts::new("vibemq_messages_sent_total", "Total messages sent by type"),
            &["type"],
        )
        .unwrap();

        let messages_bytes_received = IntCounter::with_opts(Opts::new(
            "vibemq_messages_bytes_received_total",
            "Total bytes received from clients",
        ))
        .unwrap();

        let messages_bytes_sent = IntCounter::with_opts(Opts::new(
            "vibemq_messages_bytes_sent_total",
            "Total bytes sent to clients",
        ))
        .unwrap();

        // Subscription metrics
        let subscriptions_current = IntGauge::with_opts(Opts::new(
            "vibemq_subscriptions_current",
            "Current number of active subscriptions",
        ))
        .unwrap();

        let subscriptions_total = IntCounter::with_opts(Opts::new(
            "vibemq_subscriptions_total",
            "Total subscriptions created since startup",
        ))
        .unwrap();

        let unsubscriptions_total = IntCounter::with_opts(Opts::new(
            "vibemq_unsubscriptions_total",
            "Total unsubscriptions since startup",
        ))
        .unwrap();

        // Retained messages
        let retained_messages_current = IntGauge::with_opts(Opts::new(
            "vibemq_retained_messages_current",
            "Current number of retained messages",
        ))
        .unwrap();

        let retained_bytes_current = IntGauge::with_opts(Opts::new(
            "vibemq_retained_bytes_current",
            "Current bytes used by retained messages",
        ))
        .unwrap();

        // QoS metrics
        let inflight_messages = IntGaugeVec::new(
            Opts::new(
                "vibemq_inflight_messages",
                "Current inflight messages by QoS level",
            ),
            &["qos"],
        )
        .unwrap();

        let qos1_retransmits = IntCounter::with_opts(Opts::new(
            "vibemq_qos1_retransmits_total",
            "Total QoS 1 message retransmissions",
        ))
        .unwrap();

        let qos2_retransmits = IntCounter::with_opts(Opts::new(
            "vibemq_qos2_retransmits_total",
            "Total QoS 2 message retransmissions",
        ))
        .unwrap();

        // Cluster metrics
        let cluster_peers_current = IntGauge::with_opts(Opts::new(
            "vibemq_cluster_peers_current",
            "Current number of connected cluster peers",
        ))
        .unwrap();

        let cluster_messages_forwarded = IntCounter::with_opts(Opts::new(
            "vibemq_cluster_messages_forwarded_total",
            "Total messages forwarded to cluster peers",
        ))
        .unwrap();

        let cluster_messages_received = IntCounter::with_opts(Opts::new(
            "vibemq_cluster_messages_received_total",
            "Total messages received from cluster peers",
        ))
        .unwrap();

        // Performance metrics
        let publish_latency = Histogram::with_opts(
            HistogramOpts::new(
                "vibemq_publish_latency_seconds",
                "Time to process and deliver a publish message",
            )
            .buckets(vec![
                0.0001, 0.0005, 0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0,
            ]),
        )
        .unwrap();

        let connect_duration = Histogram::with_opts(
            HistogramOpts::new(
                "vibemq_connect_duration_seconds",
                "Time to process a connect handshake",
            )
            .buckets(vec![
                0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5,
            ]),
        )
        .unwrap();

        // Register all metrics
        registry
            .register(Box::new(connections_total.clone()))
            .unwrap();
        registry
            .register(Box::new(connections_current.clone()))
            .unwrap();
        registry
            .register(Box::new(connections_by_protocol.clone()))
            .unwrap();
        registry
            .register(Box::new(messages_received_total.clone()))
            .unwrap();
        registry
            .register(Box::new(messages_sent_total.clone()))
            .unwrap();
        registry
            .register(Box::new(messages_bytes_received.clone()))
            .unwrap();
        registry
            .register(Box::new(messages_bytes_sent.clone()))
            .unwrap();
        registry
            .register(Box::new(subscriptions_current.clone()))
            .unwrap();
        registry
            .register(Box::new(subscriptions_total.clone()))
            .unwrap();
        registry
            .register(Box::new(unsubscriptions_total.clone()))
            .unwrap();
        registry
            .register(Box::new(retained_messages_current.clone()))
            .unwrap();
        registry
            .register(Box::new(retained_bytes_current.clone()))
            .unwrap();
        registry
            .register(Box::new(inflight_messages.clone()))
            .unwrap();
        registry
            .register(Box::new(qos1_retransmits.clone()))
            .unwrap();
        registry
            .register(Box::new(qos2_retransmits.clone()))
            .unwrap();
        registry
            .register(Box::new(cluster_peers_current.clone()))
            .unwrap();
        registry
            .register(Box::new(cluster_messages_forwarded.clone()))
            .unwrap();
        registry
            .register(Box::new(cluster_messages_received.clone()))
            .unwrap();
        registry
            .register(Box::new(publish_latency.clone()))
            .unwrap();
        registry
            .register(Box::new(connect_duration.clone()))
            .unwrap();

        Metrics {
            registry,
            connections_total,
            connections_current,
            connections_by_protocol,
            messages_received_total,
            messages_sent_total,
            messages_bytes_received,
            messages_bytes_sent,
            subscriptions_current,
            subscriptions_total,
            unsubscriptions_total,
            retained_messages_current,
            retained_bytes_current,
            inflight_messages,
            qos1_retransmits,
            qos2_retransmits,
            cluster_peers_current,
            cluster_messages_forwarded,
            cluster_messages_received,
            publish_latency,
            connect_duration,
        }
    }

    // Helper methods for common operations

    pub fn client_connected(&self, protocol: &str) {
        self.connections_total.inc();
        self.connections_current.inc();
        self.connections_by_protocol
            .with_label_values(&[protocol])
            .inc();
    }

    pub fn client_disconnected(&self, protocol: &str) {
        self.connections_current.dec();
        self.connections_by_protocol
            .with_label_values(&[protocol])
            .dec();
    }

    pub fn message_received(&self, msg_type: &str, bytes: usize) {
        self.messages_received_total
            .with_label_values(&[msg_type])
            .inc();
        self.messages_bytes_received.inc_by(bytes as u64);
    }

    pub fn message_sent(&self, msg_type: &str, bytes: usize) {
        self.messages_sent_total
            .with_label_values(&[msg_type])
            .inc();
        self.messages_bytes_sent.inc_by(bytes as u64);
    }

    pub fn subscription_added(&self) {
        self.subscriptions_current.inc();
        self.subscriptions_total.inc();
    }

    pub fn subscription_removed(&self) {
        self.subscriptions_current.dec();
        self.unsubscriptions_total.inc();
    }

    pub fn retained_message_stored(&self, bytes: usize) {
        self.retained_messages_current.inc();
        self.retained_bytes_current.add(bytes as i64);
    }

    pub fn retained_message_removed(&self, bytes: usize) {
        self.retained_messages_current.dec();
        self.retained_bytes_current.sub(bytes as i64);
    }

    pub fn cluster_peer_connected(&self) {
        self.cluster_peers_current.inc();
    }

    pub fn cluster_peer_disconnected(&self) {
        self.cluster_peers_current.dec();
    }

    pub fn cluster_message_forwarded(&self) {
        self.cluster_messages_forwarded.inc();
    }

    pub fn cluster_message_received(&self) {
        self.cluster_messages_received.inc();
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

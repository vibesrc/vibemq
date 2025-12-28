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
    pub connections_maximum: IntGauge,
    pub connections_by_protocol: IntGaugeVec,

    // Session metrics
    pub sessions_expired_total: IntCounter,

    // Message metrics (all packet types)
    pub messages_total_received: IntCounter,
    pub messages_total_sent: IntCounter,

    // Message metrics (by type, for Prometheus labels)
    pub messages_received_total: IntCounterVec,
    pub messages_sent_total: IntCounterVec,
    pub messages_bytes_received: IntCounter,
    pub messages_bytes_sent: IntCounter,

    // Publish-specific metrics
    pub publish_messages_received: IntCounter,
    pub publish_messages_sent: IntCounter,
    pub publish_messages_dropped: IntCounter,

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

    // DoS protection metrics
    pub connections_rejected_total: IntCounterVec,
    pub ips_banned_current: IntGauge,
    pub ips_tracked_current: IntGauge,
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

        let connections_maximum = IntGauge::with_opts(Opts::new(
            "vibemq_connections_maximum",
            "Maximum concurrent connections since startup",
        ))
        .unwrap();

        // Session metrics
        let sessions_expired_total = IntCounter::with_opts(Opts::new(
            "vibemq_sessions_expired_total",
            "Total sessions expired since startup",
        ))
        .unwrap();

        // Message metrics (all packet types)
        let messages_total_received = IntCounter::with_opts(Opts::new(
            "vibemq_messages_total_received",
            "Total packets received (all types)",
        ))
        .unwrap();

        let messages_total_sent = IntCounter::with_opts(Opts::new(
            "vibemq_messages_total_sent",
            "Total packets sent (all types)",
        ))
        .unwrap();

        // Publish-specific metrics
        let publish_messages_received = IntCounter::with_opts(Opts::new(
            "vibemq_publish_messages_received_total",
            "Total PUBLISH packets received",
        ))
        .unwrap();

        let publish_messages_sent = IntCounter::with_opts(Opts::new(
            "vibemq_publish_messages_sent_total",
            "Total PUBLISH packets sent",
        ))
        .unwrap();

        let publish_messages_dropped = IntCounter::with_opts(Opts::new(
            "vibemq_publish_messages_dropped_total",
            "Total PUBLISH messages dropped due to queue overflow",
        ))
        .unwrap();

        // Message metrics (by type, for Prometheus labels)
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

        // DoS protection metrics
        let connections_rejected_total = IntCounterVec::new(
            Opts::new(
                "vibemq_connections_rejected_total",
                "Total connections rejected by DoS protection",
            ),
            &["reason"],
        )
        .unwrap();

        let ips_banned_current = IntGauge::with_opts(Opts::new(
            "vibemq_ips_banned_current",
            "Current number of IPs banned by flapping detection",
        ))
        .unwrap();

        let ips_tracked_current = IntGauge::with_opts(Opts::new(
            "vibemq_ips_tracked_current",
            "Current number of IPs being tracked for rate limiting",
        ))
        .unwrap();

        // Register all metrics
        registry
            .register(Box::new(connections_total.clone()))
            .unwrap();
        registry
            .register(Box::new(connections_current.clone()))
            .unwrap();
        registry
            .register(Box::new(connections_maximum.clone()))
            .unwrap();
        registry
            .register(Box::new(connections_by_protocol.clone()))
            .unwrap();
        registry
            .register(Box::new(sessions_expired_total.clone()))
            .unwrap();
        registry
            .register(Box::new(messages_total_received.clone()))
            .unwrap();
        registry
            .register(Box::new(messages_total_sent.clone()))
            .unwrap();
        registry
            .register(Box::new(publish_messages_received.clone()))
            .unwrap();
        registry
            .register(Box::new(publish_messages_sent.clone()))
            .unwrap();
        registry
            .register(Box::new(publish_messages_dropped.clone()))
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
        registry
            .register(Box::new(connections_rejected_total.clone()))
            .unwrap();
        registry
            .register(Box::new(ips_banned_current.clone()))
            .unwrap();
        registry
            .register(Box::new(ips_tracked_current.clone()))
            .unwrap();

        Metrics {
            registry,
            connections_total,
            connections_current,
            connections_maximum,
            connections_by_protocol,
            sessions_expired_total,
            messages_total_received,
            messages_total_sent,
            messages_received_total,
            messages_sent_total,
            messages_bytes_received,
            messages_bytes_sent,
            publish_messages_received,
            publish_messages_sent,
            publish_messages_dropped,
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
            connections_rejected_total,
            ips_banned_current,
            ips_tracked_current,
        }
    }

    // Helper methods for common operations

    pub fn client_connected(&self, protocol: &str) {
        self.connections_total.inc();
        self.connections_current.inc();
        self.connections_by_protocol
            .with_label_values(&[protocol])
            .inc();
        // Update maximum if current exceeds it
        let current = self.connections_current.get();
        let max = self.connections_maximum.get();
        if current > max {
            self.connections_maximum.set(current);
        }
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

    // Publish-specific helpers

    pub fn publish_received(&self, bytes: usize) {
        self.publish_messages_received.inc();
        self.messages_total_received.inc();
        self.messages_bytes_received.inc_by(bytes as u64);
    }

    pub fn publish_sent(&self, bytes: usize) {
        self.publish_messages_sent.inc();
        self.messages_total_sent.inc();
        self.messages_bytes_sent.inc_by(bytes as u64);
    }

    pub fn publish_dropped(&self) {
        self.publish_messages_dropped.inc();
    }

    // Packet helpers (for total message counts)

    pub fn packet_received(&self) {
        self.messages_total_received.inc();
    }

    pub fn packet_sent(&self) {
        self.messages_total_sent.inc();
    }

    // Session helpers

    pub fn session_expired(&self) {
        self.sessions_expired_total.inc();
    }

    // DoS protection helpers

    pub fn connection_rejected(&self, reason: &str) {
        self.connections_rejected_total
            .with_label_values(&[reason])
            .inc();
    }

    pub fn update_flapping_stats(&self, banned_ips: usize, tracked_ips: usize) {
        self.ips_banned_current.set(banned_ips as i64);
        self.ips_tracked_current.set(tracked_ips as i64);
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

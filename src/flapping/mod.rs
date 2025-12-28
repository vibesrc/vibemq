//! Flapping Detection and Connection Rate Limiting
//!
//! Provides DoS protection by detecting and banning clients that:
//! - Rapidly connect and disconnect (flapping)
//! - Exceed connection rate limits
//! - Exceed concurrent connection limits per IP
//!
//! Uses the real client IP from PROXY protocol when available.

use std::net::IpAddr;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use ipnet::IpNet;
use serde::Deserialize;
use tracing::{debug, info, warn};

/// Reason for rejecting a connection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectionReason {
    /// IP is banned (static or temporary)
    Banned,
    /// Connection rate limit exceeded
    RateLimited,
    /// Maximum connections per IP exceeded
    MaxConnectionsExceeded,
}

impl RejectionReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            RejectionReason::Banned => "banned",
            RejectionReason::RateLimited => "rate_limited",
            RejectionReason::MaxConnectionsExceeded => "max_connections",
        }
    }
}

/// Flapping detection configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct FlappingConfig {
    /// Enable flapping detection
    pub enabled: bool,
    /// Maximum disconnections in window before ban
    pub max_count: u32,
    /// Detection window (e.g., "1m", "60s")
    #[serde(with = "humantime_serde")]
    pub window_time: Duration,
    /// Ban duration (e.g., "5m", "300s")
    #[serde(with = "humantime_serde")]
    pub ban_time: Duration,
}

impl Default for FlappingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_count: 15,
            window_time: Duration::from_secs(60),
            ban_time: Duration::from_secs(300),
        }
    }
}

/// Connection rate limiting configuration
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ConnectionLimitConfig {
    /// Maximum concurrent connections per IP (0 = unlimited)
    pub max_connections_per_ip: usize,
    /// Maximum new connections per second per IP
    pub rate_limit: u32,
    /// Burst allowance for rate limiting
    pub rate_burst: u32,
    /// Static banned IP addresses
    #[serde(default)]
    pub banned_ips: Vec<IpAddr>,
    /// Static allowed IP addresses (bypass all limits)
    #[serde(default)]
    pub allowed_ips: Vec<IpAddr>,
    /// Banned CIDR ranges
    #[serde(default)]
    pub banned_cidrs: Vec<String>,
    /// Allowed CIDR ranges (bypass all limits)
    #[serde(default)]
    pub allowed_cidrs: Vec<String>,
    /// Cleanup interval (e.g., "1m", "60s")
    #[serde(with = "humantime_serde")]
    pub cleanup_interval: Duration,
}

impl Default for ConnectionLimitConfig {
    fn default() -> Self {
        Self {
            max_connections_per_ip: 0, // 0 = unlimited
            rate_limit: 0,             // 0 = disabled
            rate_burst: 20,
            banned_ips: vec![],
            allowed_ips: vec![],
            banned_cidrs: vec![],
            allowed_cidrs: vec![],
            cleanup_interval: Duration::from_secs(60),
        }
    }
}

/// Per-IP tracking state
struct IpState {
    /// Current connection count
    connection_count: AtomicU32,
    /// Token bucket: tokens remaining
    tokens: AtomicU32,
    /// Last token refill timestamp (millis since tracker start)
    last_refill_ms: AtomicU64,
    /// Disconnection count in current window
    disconnect_count: AtomicU32,
    /// Window start time (millis since tracker start)
    window_start_ms: AtomicU64,
    /// First seen time for cleanup
    first_seen: Instant,
}

impl IpState {
    fn new(burst: u32, now_ms: u64) -> Self {
        Self {
            connection_count: AtomicU32::new(0),
            tokens: AtomicU32::new(burst),
            last_refill_ms: AtomicU64::new(now_ms),
            disconnect_count: AtomicU32::new(0),
            window_start_ms: AtomicU64::new(now_ms),
            first_seen: Instant::now(),
        }
    }

    /// Try to consume a token for rate limiting
    /// Returns true if allowed, false if rate limited
    fn try_consume_token(&self, rate_per_sec: u32, burst: u32, now_ms: u64) -> bool {
        // Refill tokens based on time elapsed
        let last = self.last_refill_ms.load(Ordering::Relaxed);
        let elapsed_ms = now_ms.saturating_sub(last);

        if elapsed_ms > 0 {
            // Calculate tokens to add (rate_per_sec tokens per 1000ms)
            let tokens_to_add = (elapsed_ms * rate_per_sec as u64) / 1000;

            if tokens_to_add > 0 {
                // Try to update last_refill time
                if self
                    .last_refill_ms
                    .compare_exchange(last, now_ms, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
                {
                    // Successfully claimed the refill, add tokens
                    loop {
                        let current = self.tokens.load(Ordering::Relaxed);
                        let new_tokens = (current as u64 + tokens_to_add).min(burst as u64) as u32;
                        if self
                            .tokens
                            .compare_exchange(
                                current,
                                new_tokens,
                                Ordering::Relaxed,
                                Ordering::Relaxed,
                            )
                            .is_ok()
                        {
                            break;
                        }
                    }
                }
            }
        }

        // Try to consume one token
        loop {
            let current = self.tokens.load(Ordering::Relaxed);
            if current == 0 {
                return false;
            }
            if self
                .tokens
                .compare_exchange(current, current - 1, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return true;
            }
        }
    }

    /// Record a disconnection and check for flapping
    /// Returns true if the IP should be banned for flapping
    fn record_disconnect(&self, max_count: u32, window_ms: u64, now_ms: u64) -> bool {
        let window_start = self.window_start_ms.load(Ordering::Relaxed);

        // Check if we're still in the same window
        if now_ms.saturating_sub(window_start) >= window_ms {
            // Start a new window
            self.window_start_ms.store(now_ms, Ordering::Relaxed);
            self.disconnect_count.store(1, Ordering::Relaxed);
            false
        } else {
            // Same window, increment count
            let count = self.disconnect_count.fetch_add(1, Ordering::Relaxed) + 1;
            count >= max_count
        }
    }
}

/// Flapping detector and connection rate limiter
pub struct FlappingDetector {
    /// Flapping detection config
    flapping_config: FlappingConfig,
    /// Connection limit config
    limit_config: ConnectionLimitConfig,
    /// Per-IP state tracking
    ip_state: DashMap<IpAddr, IpState>,
    /// Temporarily banned IPs (IP -> ban expiry time in ms since start)
    temp_bans: DashMap<IpAddr, u64>,
    /// Parsed banned CIDR ranges
    banned_cidrs: Vec<IpNet>,
    /// Parsed allowed CIDR ranges
    allowed_cidrs: Vec<IpNet>,
    /// Tracker start time for relative timestamps
    start_time: Instant,
}

impl FlappingDetector {
    /// Create a new flapping detector
    pub fn new(flapping_config: FlappingConfig, limit_config: ConnectionLimitConfig) -> Self {
        // Parse CIDR ranges
        let banned_cidrs: Vec<IpNet> = limit_config
            .banned_cidrs
            .iter()
            .filter_map(|s| {
                s.parse().ok().or_else(|| {
                    warn!("Invalid banned CIDR: {}", s);
                    None
                })
            })
            .collect();

        let allowed_cidrs: Vec<IpNet> = limit_config
            .allowed_cidrs
            .iter()
            .filter_map(|s| {
                s.parse().ok().or_else(|| {
                    warn!("Invalid allowed CIDR: {}", s);
                    None
                })
            })
            .collect();

        info!(
            "FlappingDetector initialized: flapping={} (max_count={}, window={:?}, ban={:?}), \
             rate_limit={}/s, burst={}, max_per_ip={}, banned_ips={}, allowed_ips={}, \
             banned_cidrs={}, allowed_cidrs={}",
            flapping_config.enabled,
            flapping_config.max_count,
            flapping_config.window_time,
            flapping_config.ban_time,
            limit_config.rate_limit,
            limit_config.rate_burst,
            limit_config.max_connections_per_ip,
            limit_config.banned_ips.len(),
            limit_config.allowed_ips.len(),
            banned_cidrs.len(),
            allowed_cidrs.len(),
        );

        Self {
            flapping_config,
            limit_config,
            ip_state: DashMap::new(),
            temp_bans: DashMap::new(),
            banned_cidrs,
            allowed_cidrs,
            start_time: Instant::now(),
        }
    }

    /// Get current time in milliseconds since start
    fn now_ms(&self) -> u64 {
        self.start_time.elapsed().as_millis() as u64
    }

    /// Check if an IP is in the static allow list
    fn is_allowed(&self, ip: IpAddr) -> bool {
        // Check static allow list
        if self.limit_config.allowed_ips.contains(&ip) {
            return true;
        }

        // Check allowed CIDR ranges
        for cidr in &self.allowed_cidrs {
            if cidr.contains(&ip) {
                return true;
            }
        }

        false
    }

    /// Check if an IP is in the static ban list
    fn is_static_banned(&self, ip: IpAddr) -> bool {
        // Check static ban list
        if self.limit_config.banned_ips.contains(&ip) {
            return true;
        }

        // Check banned CIDR ranges
        for cidr in &self.banned_cidrs {
            if cidr.contains(&ip) {
                return true;
            }
        }

        false
    }

    /// Check if an IP is temporarily banned
    fn is_temp_banned(&self, ip: IpAddr, now_ms: u64) -> bool {
        if let Some(expiry) = self.temp_bans.get(&ip) {
            if now_ms < *expiry {
                return true;
            }
            // Ban expired, remove it
            drop(expiry);
            self.temp_bans.remove(&ip);
        }
        false
    }

    /// Check if a connection should be allowed
    /// Returns Ok(()) if allowed, Err(reason) if rejected
    pub fn check_connection(&self, ip: IpAddr) -> Result<(), RejectionReason> {
        // Allowed IPs bypass all checks
        if self.is_allowed(ip) {
            return Ok(());
        }

        let now_ms = self.now_ms();

        // Check static bans
        if self.is_static_banned(ip) {
            debug!("Connection from {} rejected: static ban", ip);
            return Err(RejectionReason::Banned);
        }

        // Check temporary bans
        if self.is_temp_banned(ip, now_ms) {
            debug!("Connection from {} rejected: temporary ban", ip);
            return Err(RejectionReason::Banned);
        }

        // Get or create IP state
        let state = self
            .ip_state
            .entry(ip)
            .or_insert_with(|| IpState::new(self.limit_config.rate_burst, now_ms));

        // Check rate limit
        if self.limit_config.rate_limit > 0
            && !state.try_consume_token(
                self.limit_config.rate_limit,
                self.limit_config.rate_burst,
                now_ms,
            )
        {
            debug!("Connection from {} rejected: rate limited", ip);
            return Err(RejectionReason::RateLimited);
        }

        // Check max connections per IP
        if self.limit_config.max_connections_per_ip > 0 {
            let count = state.connection_count.load(Ordering::Relaxed) as usize;
            if count >= self.limit_config.max_connections_per_ip {
                debug!(
                    "Connection from {} rejected: max connections ({}) exceeded",
                    ip, count
                );
                return Err(RejectionReason::MaxConnectionsExceeded);
            }
        }

        Ok(())
    }

    /// Record a successful connection
    pub fn record_connection(&self, ip: IpAddr) {
        if self.is_allowed(ip) {
            return;
        }

        let now_ms = self.now_ms();
        let state = self
            .ip_state
            .entry(ip)
            .or_insert_with(|| IpState::new(self.limit_config.rate_burst, now_ms));
        state.connection_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a disconnection and check for flapping
    pub fn record_disconnection(&self, ip: IpAddr) {
        if self.is_allowed(ip) {
            return;
        }

        let now_ms = self.now_ms();

        if let Some(state) = self.ip_state.get(&ip) {
            // Decrement connection count
            let prev = state.connection_count.fetch_sub(1, Ordering::Relaxed);
            if prev == 0 {
                // Shouldn't happen, but prevent underflow
                state.connection_count.store(0, Ordering::Relaxed);
            }

            // Check for flapping if enabled
            if self.flapping_config.enabled {
                let window_ms = self.flapping_config.window_time.as_millis() as u64;
                let should_ban =
                    state.record_disconnect(self.flapping_config.max_count, window_ms, now_ms);

                if should_ban {
                    let ban_expiry_ms = now_ms + self.flapping_config.ban_time.as_millis() as u64;
                    self.temp_bans.insert(ip, ban_expiry_ms);
                    warn!(
                        "IP {} banned for {:?} due to flapping ({} disconnects in {:?})",
                        ip,
                        self.flapping_config.ban_time,
                        self.flapping_config.max_count,
                        self.flapping_config.window_time
                    );
                }
            }
        }
    }

    /// Manually ban an IP for a specified duration
    pub fn ban_ip(&self, ip: IpAddr, duration: Duration) {
        let now_ms = self.now_ms();
        let expiry_ms = now_ms + duration.as_millis() as u64;
        self.temp_bans.insert(ip, expiry_ms);
        info!("IP {} manually banned for {:?}", ip, duration);
    }

    /// Unban an IP
    pub fn unban_ip(&self, ip: IpAddr) {
        if self.temp_bans.remove(&ip).is_some() {
            info!("IP {} unbanned", ip);
        }
    }

    /// Cleanup expired entries
    pub fn cleanup(&self) {
        let now_ms = self.now_ms();

        // Remove expired bans
        self.temp_bans.retain(|ip, expiry| {
            let keep = now_ms < *expiry;
            if !keep {
                debug!("Ban expired for IP {}", ip);
            }
            keep
        });

        // Remove stale IP state entries (no connections for a while)
        let stale_threshold = self.limit_config.cleanup_interval * 2;
        self.ip_state.retain(|ip, state| {
            let count = state.connection_count.load(Ordering::Relaxed);
            let is_stale = count == 0 && state.first_seen.elapsed() > stale_threshold;
            if is_stale {
                debug!("Cleaning up stale IP state for {}", ip);
            }
            !is_stale
        });
    }

    /// Get current stats
    pub fn stats(&self) -> FlappingStats {
        FlappingStats {
            tracked_ips: self.ip_state.len(),
            banned_ips: self.temp_bans.len(),
        }
    }

    /// Get cleanup interval
    pub fn cleanup_interval(&self) -> Duration {
        self.limit_config.cleanup_interval
    }
}

/// Statistics from the flapping detector
#[derive(Debug, Clone)]
pub struct FlappingStats {
    /// Number of IPs currently being tracked
    pub tracked_ips: usize,
    /// Number of IPs currently banned
    pub banned_ips: usize,
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn test_allowed_ip_bypasses_checks() {
        let flapping = FlappingConfig::default();
        let limits = ConnectionLimitConfig {
            allowed_ips: vec!["127.0.0.1".parse().unwrap()],
            max_connections_per_ip: 1,
            ..Default::default()
        };

        let detector = FlappingDetector::new(flapping, limits);
        let ip: IpAddr = "127.0.0.1".parse().unwrap();

        // Should always be allowed
        for _ in 0..10 {
            assert!(detector.check_connection(ip).is_ok());
            detector.record_connection(ip);
        }
    }

    #[test]
    fn test_banned_ip_rejected() {
        let flapping = FlappingConfig::default();
        let mut limits = ConnectionLimitConfig::default();
        limits.banned_ips = vec!["10.0.0.1".parse().unwrap()];

        let detector = FlappingDetector::new(flapping, limits);
        let ip: IpAddr = "10.0.0.1".parse().unwrap();

        assert_eq!(detector.check_connection(ip), Err(RejectionReason::Banned));
    }

    #[test]
    fn test_max_connections_per_ip() {
        let flapping = FlappingConfig::default();
        let mut limits = ConnectionLimitConfig::default();
        limits.max_connections_per_ip = 2;
        limits.rate_limit = 0; // Disable rate limiting for this test

        let detector = FlappingDetector::new(flapping, limits);
        let ip: IpAddr = "192.168.1.1".parse().unwrap();

        // First two connections should succeed
        assert!(detector.check_connection(ip).is_ok());
        detector.record_connection(ip);
        assert!(detector.check_connection(ip).is_ok());
        detector.record_connection(ip);

        // Third should fail
        assert_eq!(
            detector.check_connection(ip),
            Err(RejectionReason::MaxConnectionsExceeded)
        );

        // After disconnection, should succeed again
        detector.record_disconnection(ip);
        assert!(detector.check_connection(ip).is_ok());
    }

    #[test]
    fn test_rate_limiting() {
        let flapping = FlappingConfig::default();
        let mut limits = ConnectionLimitConfig::default();
        limits.rate_limit = 100; // 100/sec
        limits.rate_burst = 5; // Only 5 burst
        limits.max_connections_per_ip = 0; // Disable connection limit

        let detector = FlappingDetector::new(flapping, limits);
        let ip: IpAddr = "192.168.1.1".parse().unwrap();

        // Should allow burst connections
        for i in 0..5 {
            assert!(
                detector.check_connection(ip).is_ok(),
                "Connection {} should succeed",
                i
            );
        }

        // Next should be rate limited (no time has passed)
        assert_eq!(
            detector.check_connection(ip),
            Err(RejectionReason::RateLimited)
        );
    }

    #[test]
    fn test_manual_ban_unban() {
        let flapping = FlappingConfig::default();
        let limits = ConnectionLimitConfig::default();
        let detector = FlappingDetector::new(flapping, limits);
        let ip: IpAddr = "192.168.1.1".parse().unwrap();

        // Initially allowed
        assert!(detector.check_connection(ip).is_ok());

        // Ban the IP
        detector.ban_ip(ip, Duration::from_secs(60));
        assert_eq!(detector.check_connection(ip), Err(RejectionReason::Banned));

        // Unban
        detector.unban_ip(ip);
        assert!(detector.check_connection(ip).is_ok());
    }

    #[test]
    fn test_cidr_matching() {
        let flapping = FlappingConfig::default();
        let mut limits = ConnectionLimitConfig::default();
        limits.banned_cidrs = vec!["10.0.0.0/8".to_string()];
        limits.allowed_cidrs = vec!["10.0.1.0/24".to_string()];

        let detector = FlappingDetector::new(flapping, limits);

        // 10.0.1.x is allowed (more specific allow)
        let allowed_ip: IpAddr = "10.0.1.5".parse().unwrap();
        assert!(detector.check_connection(allowed_ip).is_ok());

        // 10.0.2.x is banned (falls under 10.0.0.0/8)
        let banned_ip: IpAddr = "10.0.2.5".parse().unwrap();
        assert_eq!(
            detector.check_connection(banned_ip),
            Err(RejectionReason::Banned)
        );

        // 192.168.1.1 is not in either range
        let other_ip: IpAddr = "192.168.1.1".parse().unwrap();
        assert!(detector.check_connection(other_ip).is_ok());
    }

    #[test]
    fn test_flapping_detection() {
        let flapping = FlappingConfig {
            enabled: true,
            max_count: 3,
            window_time: Duration::from_secs(60),
            ban_time: Duration::from_secs(300),
        };

        let limits = ConnectionLimitConfig::default();

        let detector = FlappingDetector::new(flapping, limits);
        let ip: IpAddr = "192.168.1.1".parse().unwrap();

        // First, establish some connections
        detector.record_connection(ip);
        detector.record_connection(ip);
        detector.record_connection(ip);

        // Disconnect rapidly - should trigger flapping after 3
        detector.record_disconnection(ip);
        assert!(detector.check_connection(ip).is_ok()); // Not banned yet

        detector.record_disconnection(ip);
        assert!(detector.check_connection(ip).is_ok()); // Not banned yet

        detector.record_disconnection(ip);
        // Now should be banned
        assert_eq!(detector.check_connection(ip), Err(RejectionReason::Banned));
    }
}

//! Prometheus-compatible metrics emission for the rekindle daemon.
//!
//! Serves `/metrics` in text exposition format on a localhost TCP socket.
//! Only binds to 127.0.0.1 — external scraping requires explicit proxy.
//!
//! Metrics are atomic (lock-free updates from any task) and prefixed
//! with `rekindle_daemon_*` for namespace isolation in multi-daemon setups.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::daemon::dispatch::DaemonContext;

/// Daemon-level metrics. All counters/gauges are atomic for lock-free updates.
#[derive(Debug)]
pub struct DaemonMetrics {
    /// Daemon state (0=stopped, 1=starting, 2=locked, 3=resuming, 4=operational, 5=degraded, 6=detached, 7=locking, 8=shutting_down).
    pub state: AtomicU64,
    /// Uptime in seconds since daemon process start.
    pub uptime_secs: AtomicU64,
    /// Active IPC connections.
    pub ipc_connections_active: AtomicU64,
    /// Total IPC requests dispatched.
    pub ipc_requests_total: AtomicU64,
    /// IPC requests that returned errors.
    pub ipc_errors_total: AtomicU64,
    /// Active DHT watches.
    pub watches_active: AtomicU64,
    /// Gossip messages received.
    pub gossip_received_total: AtomicU64,
    /// Gossip messages deduplicated (suppressed).
    pub gossip_dedup_total: AtomicU64,
    /// DM messages sent.
    pub dm_sent_total: AtomicU64,
    /// DM messages received.
    pub dm_received_total: AtomicU64,
    /// Channel messages sent.
    pub channel_sent_total: AtomicU64,
    /// Channel messages received.
    pub channel_received_total: AtomicU64,
    /// Ratchet session count.
    pub sessions_active: AtomicU64,
    /// MEK cache entries.
    pub mek_cache_entries: AtomicU64,
    /// Veilid peer count.
    pub peers_active: AtomicU64,
    /// Friend count.
    pub friends_total: AtomicU64,
    /// Community count.
    pub communities_total: AtomicU64,
    /// Skipped key sweep deletions (last hour).
    pub skipped_keys_swept: AtomicU64,
    /// Session.json flush count.
    pub session_flushes_total: AtomicU64,
    /// Process start time (unix epoch seconds).
    pub start_time_secs: AtomicU64,
    /// Bulk frames sent.
    pub bulk_frames_sent: AtomicU64,
    /// Bulk frames received.
    pub bulk_frames_received: AtomicU64,
    /// Bulk bytes sent.
    pub bulk_bytes_sent: AtomicU64,
    /// Bulk bytes received.
    pub bulk_bytes_received: AtomicU64,
    /// Active bulk transfers.
    pub bulk_transfers_active: AtomicU64,
    /// Bulk buffer pool slabs available.
    pub bulk_pool_available: AtomicU64,
    /// Bulk buffer pool slabs in flight.
    pub bulk_pool_in_flight: AtomicU64,
}

impl DaemonMetrics {
    /// Create zeroed metrics with start time set to now.
    pub fn new() -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            state: AtomicU64::new(0),
            uptime_secs: AtomicU64::new(0),
            ipc_connections_active: AtomicU64::new(0),
            ipc_requests_total: AtomicU64::new(0),
            ipc_errors_total: AtomicU64::new(0),
            watches_active: AtomicU64::new(0),
            gossip_received_total: AtomicU64::new(0),
            gossip_dedup_total: AtomicU64::new(0),
            dm_sent_total: AtomicU64::new(0),
            dm_received_total: AtomicU64::new(0),
            channel_sent_total: AtomicU64::new(0),
            channel_received_total: AtomicU64::new(0),
            sessions_active: AtomicU64::new(0),
            mek_cache_entries: AtomicU64::new(0),
            peers_active: AtomicU64::new(0),
            friends_total: AtomicU64::new(0),
            communities_total: AtomicU64::new(0),
            skipped_keys_swept: AtomicU64::new(0),
            session_flushes_total: AtomicU64::new(0),
            start_time_secs: AtomicU64::new(now),
            bulk_frames_sent: AtomicU64::new(0),
            bulk_frames_received: AtomicU64::new(0),
            bulk_bytes_sent: AtomicU64::new(0),
            bulk_bytes_received: AtomicU64::new(0),
            bulk_transfers_active: AtomicU64::new(0),
            bulk_pool_available: AtomicU64::new(0),
            bulk_pool_in_flight: AtomicU64::new(0),
        }
    }

    /// Update metrics from DaemonContext. Called by the watchdog timer (15s).
    pub fn update_from_context(&self, ctx: &DaemonContext) {
        let state = ctx.lifecycle.state() as u8;
        self.state.store(u64::from(state), Ordering::Relaxed);

        let start = self.start_time_secs.load(Ordering::Relaxed);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.uptime_secs.store(now.saturating_sub(start), Ordering::Relaxed);

        let chat = ctx.chat.read().clone();
        if let Some(ref chat) = chat {
            self.watches_active.store(chat.watch_count() as u64, Ordering::Relaxed);
            self.friends_total.store(chat.friend_count() as u64, Ordering::Relaxed);
            self.communities_total.store(chat.community_count() as u64, Ordering::Relaxed);
            self.peers_active.store(
                u64::from(chat.io().transport().peer_count()),
                Ordering::Relaxed,
            );
        }

        self.bulk_frames_sent.store(ctx.bulk_counters.frames_sent.load(Ordering::Relaxed), Ordering::Relaxed);
        self.bulk_frames_received.store(ctx.bulk_counters.frames_received.load(Ordering::Relaxed), Ordering::Relaxed);
        self.bulk_bytes_sent.store(ctx.bulk_counters.bytes_sent.load(Ordering::Relaxed), Ordering::Relaxed);
        self.bulk_bytes_received.store(ctx.bulk_counters.bytes_received.load(Ordering::Relaxed), Ordering::Relaxed);
        self.bulk_transfers_active.store(ctx.bulk_transfers.lock().active_count() as u64, Ordering::Relaxed);
    }
}

impl Default for DaemonMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Render all metrics in Prometheus text exposition format.
fn render_prometheus(m: &DaemonMetrics) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(4096);

    let gauge = |out: &mut String, name: &str, help: &str, val: u64| {
        let _ = writeln!(out, "# HELP {name} {help}\n# TYPE {name} gauge\n{name} {val}");
    };
    let counter = |out: &mut String, name: &str, help: &str, val: u64| {
        let _ = writeln!(out, "# HELP {name} {help}\n# TYPE {name} counter\n{name} {val}");
    };

    gauge(&mut out, "rekindle_daemon_state", "Daemon lifecycle state (0-8)", m.state.load(Ordering::Relaxed));
    gauge(&mut out, "rekindle_daemon_uptime_seconds", "Daemon uptime in seconds", m.uptime_secs.load(Ordering::Relaxed));
    gauge(&mut out, "rekindle_daemon_ipc_connections_active", "Active IPC connections", m.ipc_connections_active.load(Ordering::Relaxed));
    counter(&mut out, "rekindle_daemon_ipc_requests_total", "Total IPC requests dispatched", m.ipc_requests_total.load(Ordering::Relaxed));
    counter(&mut out, "rekindle_daemon_ipc_errors_total", "IPC requests that returned errors", m.ipc_errors_total.load(Ordering::Relaxed));
    gauge(&mut out, "rekindle_daemon_watches_active", "Active DHT watches", m.watches_active.load(Ordering::Relaxed));
    counter(&mut out, "rekindle_daemon_gossip_received_total", "Gossip messages received", m.gossip_received_total.load(Ordering::Relaxed));
    counter(&mut out, "rekindle_daemon_gossip_dedup_total", "Gossip messages deduplicated", m.gossip_dedup_total.load(Ordering::Relaxed));
    counter(&mut out, "rekindle_daemon_dm_sent_total", "DM messages sent", m.dm_sent_total.load(Ordering::Relaxed));
    counter(&mut out, "rekindle_daemon_dm_received_total", "DM messages received", m.dm_received_total.load(Ordering::Relaxed));
    counter(&mut out, "rekindle_daemon_channel_sent_total", "Channel messages sent", m.channel_sent_total.load(Ordering::Relaxed));
    counter(&mut out, "rekindle_daemon_channel_received_total", "Channel messages received", m.channel_received_total.load(Ordering::Relaxed));
    gauge(&mut out, "rekindle_daemon_sessions_active", "Active Triple Ratchet sessions", m.sessions_active.load(Ordering::Relaxed));
    gauge(&mut out, "rekindle_daemon_mek_cache_entries", "MEK cache entries", m.mek_cache_entries.load(Ordering::Relaxed));
    gauge(&mut out, "rekindle_daemon_peers_active", "Veilid peer count", m.peers_active.load(Ordering::Relaxed));
    gauge(&mut out, "rekindle_daemon_friends_total", "Friend count", m.friends_total.load(Ordering::Relaxed));
    gauge(&mut out, "rekindle_daemon_communities_total", "Community count", m.communities_total.load(Ordering::Relaxed));
    counter(&mut out, "rekindle_daemon_skipped_keys_swept", "Skipped keys swept (last hour)", m.skipped_keys_swept.load(Ordering::Relaxed));
    counter(&mut out, "rekindle_daemon_session_flushes_total", "Session.json flush count", m.session_flushes_total.load(Ordering::Relaxed));
    gauge(&mut out, "rekindle_daemon_start_time_seconds", "Process start time (unix epoch)", m.start_time_secs.load(Ordering::Relaxed));
    counter(&mut out, "rekindle_bulk_frames_sent_total", "Bulk frames sent", m.bulk_frames_sent.load(Ordering::Relaxed));
    counter(&mut out, "rekindle_bulk_frames_received_total", "Bulk frames received", m.bulk_frames_received.load(Ordering::Relaxed));
    counter(&mut out, "rekindle_bulk_bytes_sent_total", "Bulk bytes sent", m.bulk_bytes_sent.load(Ordering::Relaxed));
    counter(&mut out, "rekindle_bulk_bytes_received_total", "Bulk bytes received", m.bulk_bytes_received.load(Ordering::Relaxed));
    gauge(&mut out, "rekindle_bulk_transfers_active", "Active bulk transfers", m.bulk_transfers_active.load(Ordering::Relaxed));
    gauge(&mut out, "rekindle_bulk_pool_available", "Bulk buffer pool slabs available", m.bulk_pool_available.load(Ordering::Relaxed));
    gauge(&mut out, "rekindle_bulk_pool_in_flight", "Bulk buffer pool slabs in flight", m.bulk_pool_in_flight.load(Ordering::Relaxed));

    out
}

/// Spawn a Prometheus metrics HTTP server on `127.0.0.1:port`.
///
/// Serves `/metrics` in text exposition format. Binds only to localhost
/// to prevent external scraping without explicit proxy configuration.
/// Returns when the listener is cancelled (daemon shutdown).
pub async fn serve_prometheus(metrics: Arc<DaemonMetrics>, port: u16) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let addr = format!("127.0.0.1:{port}");
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => {
            tracing::info!(addr = %addr, "Prometheus metrics endpoint listening");
            l
        }
        Err(e) => {
            tracing::warn!(
                addr = %addr,
                error = %e,
                "failed to bind Prometheus metrics endpoint — metrics disabled. \
                 Another process may be using port {port}.",
            );
            return;
        }
    };

    loop {
        let (mut stream, _) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::debug!(error = %e, "metrics accept error");
                continue;
            }
        };

        let metrics_clone = Arc::clone(&metrics);
        tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(&mut stream);
            let mut request_line = String::new();
            let _ = reader.read_line(&mut request_line).await;

            let response = if request_line.starts_with("GET /metrics") {
                let body = render_prometheus(&metrics_clone);
                format!(
                    "HTTP/1.1 200 OK\r\n\
                     Content-Type: text/plain; version=0.0.4; charset=utf-8\r\n\
                     Content-Length: {}\r\n\
                     Connection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
            } else if request_line.starts_with("GET /health") {
                "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_string()
            } else {
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_string()
            };

            let _ = stream.write_all(response.as_bytes()).await;
        });
    }
}

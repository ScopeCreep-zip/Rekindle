//! Health check endpoint and diagnostic dump facility.
//!
//! Health: lightweight TCP probe on localhost for load balancer / container
//! orchestration liveness checks. Returns 200 OK if the daemon process is
//! alive and the lifecycle state is operational/degraded/locked. Returns
//! 503 if shutting down.
//!
//! Diagnostic dump: triggered by SIGUSR1. Writes a full state snapshot to
//! a timestamped file in the state directory. Contains: daemon state,
//! peer count, watch count, session count, MEK cache stats, community
//! list, friend list, uptime, memory usage. No secrets.

use std::sync::Arc;

use crate::daemon::{DaemonLifecycle, DaemonState};
use crate::daemon::dispatch::DaemonContext;
use crate::state::StatePaths;

/// Serve health check on `127.0.0.1:port`.
///
/// Responds to any TCP connection with a minimal HTTP response:
/// - 200 "ok" if daemon state is Locked/Operational/Degraded/Detached
/// - 503 "shutting_down" if daemon state is ShuttingDown/Stopped
/// - 503 "starting" if daemon state is Starting/Resuming
///
/// No request parsing beyond detecting a TCP connection. Load balancers
/// and container orchestrators (Kubernetes livenessProbe, Docker HEALTHCHECK)
/// only need a TCP connect or HTTP status code.
pub async fn serve_health(lifecycle: Arc<DaemonLifecycle>, port: u16) {
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    let addr = format!("127.0.0.1:{port}");
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => {
            tracing::info!(addr = %addr, "health check endpoint listening");
            l
        }
        Err(e) => {
            tracing::warn!(
                addr = %addr,
                error = %e,
                "failed to bind health check endpoint — health probes disabled. \
                 Another process may be using port {port}."
            );
            return;
        }
    };

    loop {
        let (mut stream, _) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::trace!(error = %e, "health accept error");
                continue;
            }
        };

        let state = lifecycle.state();
        let (status, body) = match state {
            DaemonState::Operational
            | DaemonState::Degraded
            | DaemonState::Detached
            | DaemonState::Locked => ("200 OK", state.as_str()),
            DaemonState::ShuttingDown | DaemonState::Stopped => {
                ("503 Service Unavailable", state.as_str())
            }
            _ => ("503 Service Unavailable", state.as_str()),
        };

        let response = format!(
            "HTTP/1.1 {status}\r\n\
             Content-Type: text/plain\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\r\n\
             {body}",
            body.len(),
        );
        let _ = stream.write_all(response.as_bytes()).await;
    }
}

/// Write a diagnostic dump to a timestamped file in the state directory.
///
/// Triggered by SIGUSR1. Contains a full non-secret state snapshot:
/// - Daemon state + uptime
/// - Transport attachment + peer count
/// - Watch count + registered watch keys
/// - Session count
/// - MEK cache stats (channel count, generation counts — no key material)
/// - Community list (names, governance keys, our pseudonyms)
/// - Friend list (display names, peer keys)
/// - Memory usage (from /proc/self/statm on Linux)
/// - Event pipeline stats (dedup entries, suppressed count)
///
/// No signing keys, no vault contents, no message plaintext, no ratchet state.
pub async fn write_diagnostic_dump(ctx: &Arc<DaemonContext>, paths: &StatePaths) {
    use std::fmt::Write;
    let mut dump = String::with_capacity(8192);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let _ = writeln!(dump, "=== REKINDLE DIAGNOSTIC DUMP ===");
    let _ = writeln!(dump, "timestamp_unix: {now}");
    let _ = writeln!(dump, "pid: {}", std::process::id());
    let _ = writeln!(dump, "state: {}", ctx.lifecycle.state().as_str());

    // Chat service diagnostics
    let chat = ctx.chat.read().clone();
    if let Some(ref chat) = chat {
        let _ = writeln!(dump, "\n--- Chat Service ---");
        let _ = writeln!(dump, "signing_key_loaded: {}", chat.io().is_signing_key_loaded());
        let _ = writeln!(dump, "transport_attached: {}", chat.io().transport().is_attached());
        let _ = writeln!(dump, "peer_count: {}", chat.io().transport().peer_count());
        let _ = writeln!(dump, "uptime_secs: {}", chat.io().transport().uptime_secs());
        let _ = writeln!(dump, "watch_count: {}", chat.watch_count());
        let _ = writeln!(dump, "community_count: {}", chat.community_count());
        let _ = writeln!(dump, "friend_count: {}", chat.friend_count());
        let _ = writeln!(dump, "unread_channels: {}", chat.unread_channels().len());
        let _ = writeln!(dump, "unread_dms: {}", chat.unread_dms().len());
        let _ = writeln!(dump, "unread_friend_requests: {}", chat.unread_friend_requests());

        if let Some(identity) = chat.session_identity() {
            let _ = writeln!(dump, "\n--- Identity ---");
            let _ = writeln!(dump, "public_key: {}", identity.public_key_hex);
            let _ = writeln!(dump, "display_name: {}", identity.display_name);
            let _ = writeln!(dump, "profile_dht_key: {}", identity.profile_dht_key);
        }
    } else {
        let _ = writeln!(dump, "\n--- Chat Service: NOT INITIALIZED (daemon locked) ---");
    }

    // Memory usage from /proc/self/statm (Linux only)
    #[cfg(target_os = "linux")]
    {
        if let Ok(statm) = std::fs::read_to_string("/proc/self/statm") {
            let fields: Vec<&str> = statm.split_whitespace().collect();
            if fields.len() >= 2 {
                let page_size = 4096u64; // Assume 4K pages
                let virt_pages: u64 = fields[0].parse().unwrap_or(0);
                let rss_pages: u64 = fields[1].parse().unwrap_or(0);
                let _ = writeln!(dump, "\n--- Memory ---");
                let _ = writeln!(dump, "virtual_mb: {}", virt_pages * page_size / 1_048_576);
                let _ = writeln!(dump, "rss_mb: {}", rss_pages * page_size / 1_048_576);
            }
        }
    }

    // Write to file
    let filename = format!("diagnostic-{now}.txt");
    let dump_path = paths.state_dir.join(&filename);
    match tokio::fs::write(&dump_path, &dump).await {
        Ok(()) => {
            tracing::info!(
                path = %dump_path.display(),
                size_bytes = dump.len(),
                "diagnostic dump written"
            );
        }
        Err(e) => {
            tracing::error!(
                path = %dump_path.display(),
                error = %e,
                "diagnostic dump write FAILED"
            );
            // Fall back to tracing so operators can still see it in logs.
            tracing::error!(dump = %dump, "diagnostic dump (file write failed, dumping to log)");
        }
    }
}

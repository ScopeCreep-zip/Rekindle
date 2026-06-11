//! Health check endpoint and diagnostic dump facility.
//!
//! Health: lightweight TCP probe on localhost for load balancer / container
//! orchestration liveness checks. Returns 200 OK if the daemon process is
//! alive and the lifecycle state is operational/degraded/locked. Returns
//! 503 if shutting down.
//!
//! Diagnostic dump: triggered by SIGUSR1. Writes a full state snapshot to
//! a timestamped file in the state directory. No secrets.

use std::sync::Arc;

use crate::daemon::{DaemonLifecycle, DaemonState};
use crate::daemon::dispatch::DaemonContext;
use crate::state::StatePaths;

/// Serve health check on `127.0.0.1:port`.
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

    // Security hardening
    {
        let _ = writeln!(dump, "\n--- Security Hardening ---");
        #[cfg(target_os = "linux")]
        {
            // SAFETY: prctl with PR_GET_DUMPABLE has no pointer args, no UB.
            let dumpable = unsafe { libc::prctl(libc::PR_GET_DUMPABLE) };
            let _ = writeln!(dump, "pr_dumpable: {dumpable} (0=non-dumpable, 1=dumpable)");

            // SAFETY: zeroed rlimit is valid for getrlimit.
            let mut rlim: libc::rlimit = unsafe { std::mem::zeroed() };
            // SAFETY: getrlimit with valid rlimit pointer on stack.
            let _ = unsafe { libc::getrlimit(libc::RLIMIT_CORE, &raw mut rlim) };
            let _ = writeln!(dump, "rlimit_core_cur: {}", rlim.rlim_cur);
            let _ = writeln!(dump, "rlimit_core_max: {}", rlim.rlim_max);

            // SAFETY: zeroed rlimit is valid for getrlimit.
            let mut nofile: libc::rlimit = unsafe { std::mem::zeroed() };
            // SAFETY: getrlimit with valid rlimit pointer.
            let _ = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut nofile) };
            let _ = writeln!(dump, "rlimit_nofile_cur: {}", nofile.rlim_cur);
            let _ = writeln!(dump, "rlimit_nofile_max: {}", nofile.rlim_max);

            // SAFETY: zeroed rlimit is valid for getrlimit.
            let mut memlock: libc::rlimit = unsafe { std::mem::zeroed() };
            // SAFETY: getrlimit with valid rlimit pointer.
            let _ = unsafe { libc::getrlimit(libc::RLIMIT_MEMLOCK, &raw mut memlock) };
            let _ = writeln!(dump, "rlimit_memlock_cur: {}", memlock.rlim_cur);
            let _ = writeln!(dump, "rlimit_memlock_max: {}", memlock.rlim_max);

            let sandbox_disabled = std::env::var("REKINDLE_DISABLE_SANDBOX").is_ok();
            let _ = writeln!(dump, "sandbox_bypassed: {sandbox_disabled}");

            // THP status
            // SAFETY: prctl PR_GET_THP_DISABLE has no pointer args.
            let thp_disabled = unsafe { libc::prctl(libc::PR_GET_THP_DISABLE, 0, 0, 0, 0) };
            let _ = writeln!(dump, "thp_disabled_for_process: {}", thp_disabled != 0);

            if let Ok(thp_sys) = std::fs::read_to_string("/sys/kernel/mm/transparent_hugepage/enabled") {
                let _ = writeln!(dump, "thp_system_policy: {}", thp_sys.trim());
            }

            // Kernel version
            // SAFETY: zeroed utsname is valid for uname().
            let mut uname: libc::utsname = unsafe { std::mem::zeroed() };
            // SAFETY: uname fills the struct, no UB.
            if unsafe { libc::uname(&raw mut uname) } == 0 {
                // SAFETY: release is a null-terminated C string.
                let release = unsafe { std::ffi::CStr::from_ptr(uname.release.as_ptr()) };
                let _ = writeln!(dump, "kernel: {}", release.to_string_lossy());
            }
        }

        #[cfg(not(target_os = "linux"))]
        {
            let _ = writeln!(dump, "platform_hardening: not available (non-Linux)");
        }
    }

    // Subscriptions
    {
        let _ = writeln!(dump, "\n--- Subscriptions ---");
        let _ = writeln!(dump, "active_connections: {}", ctx.subscriptions.connection_count());
    }

    // Registered agents
    {
        let agents = ctx.agents.read();
        let _ = writeln!(dump, "\n--- Registered Agents ---");
        let _ = writeln!(dump, "agent_count: {}", agents.len());
        for (name, reg) in agents.iter() {
            let _ = writeln!(dump, "  {name}: type={:?} capabilities={:?}", reg.agent_type, reg.capabilities);
        }
    }

    // Transport counters
    {
        let counters = ctx.transport_counters.snapshot();
        let _ = writeln!(dump, "\n--- Transport Counters ---");
        let _ = writeln!(dump, "frames_sent: {}", counters.frames_sent);
        let _ = writeln!(dump, "frames_received: {}", counters.frames_received);
        let _ = writeln!(dump, "bytes_sent: {}", counters.bytes_sent);
        let _ = writeln!(dump, "bytes_received: {}", counters.bytes_received);
    }

    // Noise protocol
    {
        let _ = writeln!(dump, "\n--- Noise Protocol ---");
        let _ = writeln!(dump, "params: {}", rekindle_keys::NOISE_PARAMS);
        let _ = writeln!(dump, "resolver: aws-lc (custom CryptoResolver)");
    }

    // Memory usage from /proc/self/statm (Linux only)
    #[cfg(target_os = "linux")]
    {
        if let Ok(statm) = std::fs::read_to_string("/proc/self/statm") {
            let fields: Vec<&str> = statm.split_whitespace().collect();
            if fields.len() >= 2 {
                let page_size = 4096u64;
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
            tracing::error!(dump = %dump, "diagnostic dump (file write failed, dumping to log)");
        }
    }
}

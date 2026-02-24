#![recursion_limit = "512"]

mod audit;
mod automod;
mod community_host;
mod db;
mod db_helpers;
mod identity;
mod invite_util;
mod ipc;
mod mek;
mod rpc;
mod server_state;
mod tasks;

use std::sync::Arc;

use clap::{Parser, Subcommand};
use parking_lot::RwLock;
use rekindle_protocol::node::{NodeConfig, RekindleNode};
use tokio::sync::mpsc;
use veilid_core::VeilidUpdate;

use server_state::ServerState;

#[derive(Parser)]
#[command(name = "rekindle-server", about = "Community server daemon for Rekindle")]
struct Cli {
    /// Directory for Veilid storage
    #[arg(long, default_value_t = dirs_fallback("rekindle-server/veilid"))]
    storage_dir: String,

    /// Unix socket path for IPC
    #[arg(long, default_value_t = default_socket_path())]
    socket: String,

    /// SQLite database path
    #[arg(long, default_value_t = dirs_fallback("rekindle-server/server.db"))]
    db: String,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Query server status
    Status,
    /// Generate a community invite code
    Invite {
        /// Community ID to generate invite for
        community_id: String,
        /// Maximum number of uses (unlimited if not set)
        #[arg(long)]
        max_uses: Option<u32>,
        /// Expiry duration (e.g., "24h", "7d", "30m")
        #[arg(long)]
        expires: Option<String>,
    },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Some(Command::Status) => {
            handle_status_command(&cli.socket).await;
            return;
        }
        Some(Command::Invite {
            community_id,
            max_uses,
            expires,
        }) => {
            handle_invite_command(&cli.socket, &community_id, max_uses, expires.as_deref()).await;
            return;
        }
        None => {
            // Default: start daemon
        }
    }

    tracing::info!("rekindle-server starting");

    // Ensure storage directory exists
    std::fs::create_dir_all(&cli.storage_dir).expect("failed to create storage dir");
    if let Some(parent) = std::path::Path::new(&cli.db).parent() {
        std::fs::create_dir_all(parent).expect("failed to create db dir");
    }

    // Open server database
    let db = db::open_server_db(&cli.db).expect("failed to open server database");

    // Load or create server identity
    let (identity, public_key_hex) =
        identity::load_or_create_identity(&db).expect("failed to load/create server identity");

    // Start Veilid node via rekindle-protocol's RekindleNode
    let node_config = NodeConfig {
        storage_dir: cli.storage_dir.clone(),
        app_namespace: "rekindle-server".into(),
        qualifier: "rekindle-server".into(),
    };
    let mut node = RekindleNode::start(node_config)
        .await
        .expect("failed to start Veilid node");
    let veilid_api = node.api().clone();
    let routing_context = node.routing_context().clone();
    let update_rx = node.take_update_receiver();
    // api and routing_context are Arc-based — safe to drop the node wrapper.
    // Explicit shutdown is at process exit via state.api.clone().shutdown().await.
    drop(node);

    let state = Arc::new(ServerState {
        api: veilid_api,
        routing_context,
        db,
        hosted: RwLock::new(std::collections::HashMap::new()),
        started_at: rekindle_utils::timestamp_secs(),
        identity,
        public_key_hex,
        slowmode_last_message: RwLock::new(std::collections::HashMap::new()),
        rate_limiter: automod::RateLimiter::new(10, 10),
        broadcast_listeners: RwLock::new(std::collections::HashMap::new()),
    });

    tracing::info!(public_key = %state.public_key_hex, "server identity loaded");

    // Start the DHT keep-alive loop
    let (keepalive_shutdown_tx, keepalive_shutdown_rx) = mpsc::channel(1);
    let keepalive_state = Arc::clone(&state);
    tokio::spawn(community_host::dht_keepalive_loop(
        keepalive_state,
        keepalive_shutdown_rx,
    ));

    // Start the async IPC listener as a tokio task.
    // All IPC handling is fully async — no block_on() bridges, no OS-level
    // socket timeouts. Sync handlers execute inline without yielding; async
    // handlers (HostCommunity, CommunityRpc, GetStatus) .await directly.
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);
    let ipc_state = Arc::clone(&state);
    let socket = cli.socket.clone();
    let ipc_shutdown_tx = shutdown_tx.clone();
    tokio::spawn(ipc::start_ipc_listener(
        socket,
        ipc_state,
        ipc_shutdown_tx,
    ));

    // Start the Veilid dispatch loop
    let dispatch_state = Arc::clone(&state);
    tokio::spawn(server_dispatch_loop(dispatch_state, update_rx));

    // Load previously hosted communities from DB
    load_persisted_communities(&state).await;

    // Spawn fast route recovery for communities that failed initial allocation
    let route_retry_state = Arc::clone(&state);
    tokio::spawn(retry_failed_routes(route_retry_state));

    // Broadcast channel for graceful shutdown of periodic tasks
    let (task_shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);

    // Spawn automod cleanup task (every 60s)
    let cleanup_state = Arc::clone(&state);
    let mut cleanup_shutdown = task_shutdown_tx.subscribe();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    tasks::cleanup_rate_limiter(&cleanup_state);
                    tasks::cleanup_slowmode_tracker(&cleanup_state);
                },
                _ = cleanup_shutdown.recv() => break,
            }
        }
    });

    // Spawn thread auto-archive task (every 10 min)
    let archive_state = Arc::clone(&state);
    let mut archive_shutdown = task_shutdown_tx.subscribe();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(600));
        loop {
            tokio::select! {
                _ = interval.tick() => tasks::auto_archive_stale_threads(&archive_state),
                _ = archive_shutdown.recv() => break,
            }
        }
    });

    // Spawn event lifecycle + reminder task (every 5 min)
    let reminder_state = Arc::clone(&state);
    let mut reminder_shutdown = task_shutdown_tx.subscribe();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    tasks::advance_event_lifecycle(&reminder_state);
                    tasks::cleanup_past_events(&reminder_state);
                    let reminders = tasks::check_event_reminders(&reminder_state);
                    for (community_id, event_id, title, minutes) in reminders {
                        rpc::broadcast_event_reminder(
                            &reminder_state,
                            &community_id,
                            &event_id,
                            &title,
                            minutes,
                        );
                    }
                }
                _ = reminder_shutdown.recv() => break,
            }
        }
    });

    tracing::info!(socket = %cli.socket, "rekindle-server ready");

    // Wait for shutdown signal (IPC or OS signals)
    tokio::select! {
        () = async { let _ = shutdown_rx.recv().await; } => {
            tracing::info!("shutdown requested via IPC");
        }
        () = async { let _ = tokio::signal::ctrl_c().await; } => {
            tracing::info!("received SIGINT — shutting down");
        }
        () = async {
            #[cfg(unix)]
            {
                let mut term = tokio::signal::unix::signal(
                    tokio::signal::unix::SignalKind::terminate(),
                ).expect("failed to register SIGTERM handler");
                term.recv().await;
            }
            #[cfg(not(unix))]
            {
                // On non-Unix, just pend forever (ctrl_c handles it)
                std::future::pending::<()>().await;
            }
        } => {
            tracing::info!("received SIGTERM — shutting down");
        }
    }

    tracing::info!("rekindle-server shutting down");

    // Stop periodic tasks
    let _ = task_shutdown_tx.send(());

    // Stop keep-alive loop
    let _ = keepalive_shutdown_tx.send(()).await;

    // Release all routes
    {
        let hosted = state.hosted.read();
        for community in hosted.values() {
            if let Some(ref route_id) = community.route_id {
                let _ = state.api.release_private_route(route_id.clone());
            }
        }
    }

    // Shut down Veilid node
    state.api.clone().shutdown().await;

    // Clean up socket file
    let _ = std::fs::remove_file(&cli.socket);

    tracing::info!("rekindle-server stopped");
}

/// Parse a duration string like "30m", "24h", "7d" into seconds.
fn parse_duration_to_seconds(s: &str) -> Result<u64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty duration".into());
    }
    let (num_str, unit) = s.split_at(s.len() - 1);
    let num: u64 = num_str
        .parse()
        .map_err(|_| format!("invalid number: {num_str}"))?;
    match unit {
        "s" => Ok(num),
        "m" => Ok(num * 60),
        "h" => Ok(num * 3600),
        "d" => Ok(num * 86400),
        _ => Err(format!("unknown unit: {unit} (use s/m/h/d)")),
    }
}

/// Connect to the running daemon's IPC socket and send a `GetStatus` request.
#[allow(clippy::print_stdout, clippy::print_stderr, clippy::single_match_else)]
async fn handle_status_command(socket_path: &str) {
    use ipc::{IpcRequest, IpcResponse};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    let stream = match UnixStream::connect(socket_path).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to connect to server at {socket_path}: {e}");
            eprintln!("is the server running?");
            std::process::exit(1);
        }
    };

    let (reader, mut writer) = stream.into_split();

    // Send a GetStatus request (newline-delimited JSON matching IpcRequest)
    let request = IpcRequest::GetStatus;
    let mut request_bytes = serde_json::to_vec(&request).unwrap();
    request_bytes.push(b'\n');
    if let Err(e) = writer.write_all(&request_bytes).await {
        eprintln!("failed to send request: {e}");
        std::process::exit(1);
    }
    if let Err(e) = writer.flush().await {
        eprintln!("failed to flush request: {e}");
        std::process::exit(1);
    }

    // Read the response line
    let mut lines = BufReader::new(reader).lines();
    match lines.next_line().await {
        Ok(Some(line)) => {
            let response: IpcResponse = match serde_json::from_str(&line) {
                Ok(r) => r,
                Err(_) => {
                    eprintln!("invalid response from server: {line}");
                    std::process::exit(1);
                }
            };
            match response {
                IpcResponse::Status {
                    uptime_secs,
                    community_count,
                    veilid_attached,
                    server_public_key,
                } => {
                    println!("Server Public Key: {server_public_key}");
                    println!("Uptime:           {uptime_secs}s");
                    println!("Communities:      {community_count}");
                    println!("Veilid Attached:  {veilid_attached}");
                }
                IpcResponse::Error { message } => {
                    eprintln!("server error: {message}");
                    std::process::exit(1);
                }
                other => {
                    println!("{}", serde_json::to_string_pretty(&other).unwrap_or_default());
                }
            }
        }
        Ok(None) => {
            eprintln!("server closed connection without responding");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("failed to read response: {e}");
            std::process::exit(1);
        }
    }
}

/// Connect to the running daemon's IPC socket and send a `CreateInvite` request.
#[allow(clippy::print_stdout, clippy::print_stderr, clippy::single_match_else)]
async fn handle_invite_command(
    socket_path: &str,
    community_id: &str,
    max_uses: Option<u32>,
    expires: Option<&str>,
) {
    use ipc::{IpcRequest, IpcResponse};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    let expires_in_seconds = expires.map(|e| {
        parse_duration_to_seconds(e).unwrap_or_else(|err| {
            eprintln!("invalid --expires value: {err}");
            std::process::exit(1);
        })
    });

    let stream = match UnixStream::connect(socket_path).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to connect to server at {socket_path}: {e}");
            eprintln!("is the server running?");
            std::process::exit(1);
        }
    };

    let (reader, mut writer) = stream.into_split();

    let request = IpcRequest::CreateInvite {
        community_id: community_id.to_string(),
        max_uses,
        expires_in_seconds,
    };
    let mut request_bytes = serde_json::to_vec(&request).unwrap();
    request_bytes.push(b'\n');
    if let Err(e) = writer.write_all(&request_bytes).await {
        eprintln!("failed to send request: {e}");
        std::process::exit(1);
    }
    if let Err(e) = writer.flush().await {
        eprintln!("failed to flush request: {e}");
        std::process::exit(1);
    }

    let mut lines = BufReader::new(reader).lines();
    match lines.next_line().await {
        Ok(Some(line)) => {
            let response: IpcResponse = match serde_json::from_str(&line) {
                Ok(r) => r,
                Err(_) => {
                    eprintln!("invalid response from server: {line}");
                    std::process::exit(1);
                }
            };
            match response {
                IpcResponse::InviteCreated { code, signature } => {
                    println!("Invite code: {code}");
                    println!("Signature:   {signature}");
                }
                IpcResponse::Error { message } => {
                    eprintln!("server error: {message}");
                    std::process::exit(1);
                }
                other => {
                    println!("{}", serde_json::to_string_pretty(&other).unwrap_or_default());
                }
            }
        }
        Ok(None) => {
            eprintln!("server closed connection without responding");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("failed to read response: {e}");
            std::process::exit(1);
        }
    }
}

/// Retry route allocation for communities that failed during initial setup.
///
/// When the server starts, `host_community()` may fail to allocate private routes
/// because Veilid hasn't fully attached yet. Rather than waiting for the 5-minute
/// keepalive cycle, this task retries with exponential backoff (5s, 10s, 20s, 40s, 80s)
/// and exits early once all communities have routes.
async fn retry_failed_routes(state: Arc<ServerState>) {
    use rekindle_protocol::dht::community::SUBKEY_SERVER_ROUTE;
    use rekindle_protocol::dht::DHTManager;

    let delays_secs = [5u64, 10, 20, 40, 80];

    for delay in delays_secs {
        tokio::time::sleep(std::time::Duration::from_secs(delay)).await;

        // Collect communities missing routes
        let needs_route: Vec<(String, String)> = {
            let hosted = state.hosted.read();
            hosted
                .values()
                .filter(|c| c.route_blob.is_none())
                .map(|c| (c.community_id.clone(), c.dht_record_key.clone()))
                .collect()
        };

        if needs_route.is_empty() {
            tracing::debug!("all communities have routes — route retry task exiting");
            return;
        }

        tracing::info!(
            count = needs_route.len(),
            delay_secs = delay,
            "retrying route allocation for communities without routes"
        );

        for (community_id, dht_key) in &needs_route {
            match state.api.new_private_route().await {
                Ok(rb) => {
                    tracing::info!(community = %community_id, "recovered: allocated route on retry");

                    // Update in-memory state
                    {
                        let mut hosted = state.hosted.write();
                        if let Some(c) = hosted.get_mut(community_id) {
                            c.route_id = Some(rb.route_id);
                            c.route_blob = Some(rb.blob.clone());
                        }
                    }

                    // Publish route to DHT
                    let mgr = DHTManager::new(state.routing_context.clone());
                    if let Err(e) = mgr.set_value(dht_key, SUBKEY_SERVER_ROUTE, rb.blob).await {
                        tracing::warn!(
                            error = %e,
                            community = %community_id,
                            "route allocated but failed to publish to DHT"
                        );
                    }
                }
                Err(e) => {
                    tracing::debug!(
                        error = %e,
                        community = %community_id,
                        delay_secs = delay,
                        "route allocation retry failed — will try again"
                    );
                }
            }
        }
    }

    // Check if any communities still lack routes after all retries
    let remaining: usize = {
        let hosted = state.hosted.read();
        hosted.values().filter(|c| c.route_blob.is_none()).count()
    };
    if remaining > 0 {
        tracing::warn!(
            remaining,
            "route retry exhausted — {remaining} communities still without routes, \
             falling back to 5-minute keepalive cycle"
        );
    }
}

/// Server-side Veilid dispatch loop — routes incoming events.
async fn server_dispatch_loop(
    state: Arc<ServerState>,
    mut update_rx: mpsc::Receiver<VeilidUpdate>,
) {
    while let Some(update) = update_rx.recv().await {
        match update {
            VeilidUpdate::AppCall(call) => {
                let call = *call;
                let state = Arc::clone(&state);
                let incoming_route_id = call.route_id().cloned();
                tokio::spawn(async move {
                    let response = rpc::handle_community_request(
                        &state,
                        call.message(),
                        incoming_route_id.as_ref(),
                    )
                    .await;
                    if let Err(e) = state.api.app_call_reply(call.id(), response).await {
                        tracing::error!(error = %e, "failed to send app_call reply — caller will hang");
                    }
                });
            }
            VeilidUpdate::AppMessage(msg) => {
                let _msg = *msg;
                // Future: handle member route updates, broadcast receipts
            }
            VeilidUpdate::RouteChange(change) => {
                let dead_routes: Vec<veilid_core::RouteId> = change.dead_routes;
                let dead_remote_routes: Vec<veilid_core::RouteId> = change.dead_remote_routes;
                community_host::handle_server_route_change(&state, &dead_routes).await;
                if !dead_remote_routes.is_empty() {
                    community_host::clear_dead_member_routes(&state, &dead_remote_routes);
                }
            }
            VeilidUpdate::Attachment(att) => {
                tracing::info!(state = %att.state, "server attachment changed");
            }
            _ => {}
        }
    }
}

/// Load previously hosted communities from the server database.
async fn load_persisted_communities(state: &Arc<ServerState>) {
    let communities = match load_communities_from_db(state) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "failed to load hosted communities from DB");
            return;
        }
    };

    for c in communities {
        // Pass the stored creator_pseudonym — host_community will skip
        // re-registering if the creator is already in the members table.
        if let Err(e) = community_host::host_community(
            state,
            &c.id,
            &c.dht_record_key,
            &c.owner_keypair_hex,
            &c.name,
            &c.creator_pseudonym,
            "",
        )
        .await
        {
            tracing::error!(community = %c.id, error = %e, "failed to re-host community");
        }
    }
}

/// A community record loaded from the server database.
struct PersistedCommunity {
    id: String,
    dht_record_key: String,
    owner_keypair_hex: String,
    name: String,
    creator_pseudonym: String,
}

/// Load persisted community records from the server database.
fn load_communities_from_db(
    state: &Arc<ServerState>,
) -> Result<Vec<PersistedCommunity>, String> {
    db_helpers::db_call(&state.db, |db| {
        let mut stmt = db.prepare(
            "SELECT id, dht_record_key, owner_keypair_hex, name, creator_pseudonym FROM hosted_communities",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(PersistedCommunity {
                id: row.get(0)?,
                dht_record_key: row.get(1)?,
                owner_keypair_hex: row.get(2)?,
                name: row.get(3)?,
                creator_pseudonym: row.get(4)?,
            })
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    })
}


fn dirs_fallback(subpath: &str) -> String {
    let base = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    format!("{base}/.local/share/{subpath}")
}

fn default_socket_path() -> String {
    let tmp = std::env::temp_dir();
    tmp.join("rekindle-server.sock")
        .to_string_lossy()
        .to_string()
}


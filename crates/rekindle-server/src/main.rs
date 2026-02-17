#![recursion_limit = "512"]

mod community_host;
mod db;
mod ipc;
mod mek;
mod rpc;
mod server_state;

use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::mpsc;
use veilid_core::VeilidUpdate;

use server_state::ServerState;

/// Command-line arguments for the server daemon.
struct Args {
    storage_dir: String,
    socket_path: String,
    db_path: String,
}

fn parse_args() -> Args {
    let mut args = std::env::args().skip(1);
    let mut storage_dir = String::new();
    let mut socket_path = String::new();
    let mut db_path = String::new();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--storage-dir" => storage_dir = args.next().unwrap_or_default(),
            "--socket" => socket_path = args.next().unwrap_or_default(),
            "--db" => db_path = args.next().unwrap_or_default(),
            _ => {}
        }
    }

    if storage_dir.is_empty() {
        storage_dir = dirs_fallback("rekindle-server/veilid");
    }
    if socket_path.is_empty() {
        socket_path = default_socket_path();
    }
    if db_path.is_empty() {
        db_path = dirs_fallback("rekindle-server/server.db");
    }

    Args {
        storage_dir,
        socket_path,
        db_path,
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    tracing::info!("rekindle-server starting");

    let args = parse_args();

    // Ensure storage directory exists
    std::fs::create_dir_all(&args.storage_dir).expect("failed to create storage dir");
    if let Some(parent) = std::path::Path::new(&args.db_path).parent() {
        std::fs::create_dir_all(parent).expect("failed to create db dir");
    }

    // Open server database
    let db = db::open_server_db(&args.db_path).expect("failed to open server database");

    // Start Veilid node
    let (update_tx, update_rx) = mpsc::channel::<VeilidUpdate>(1024);

    let veilid_api = start_veilid_node(&args.storage_dir, update_tx)
        .await
        .expect("failed to start Veilid node");

    let routing_context = veilid_api
        .routing_context()
        .expect("failed to create routing context");

    let state = Arc::new(ServerState {
        api: veilid_api,
        routing_context,
        db,
        hosted: RwLock::new(std::collections::HashMap::new()),
        started_at: timestamp_now_secs(),
    });

    // Start the DHT keep-alive loop
    let (keepalive_shutdown_tx, keepalive_shutdown_rx) = mpsc::channel(1);
    let keepalive_state = Arc::clone(&state);
    tokio::spawn(community_host::dht_keepalive_loop(
        keepalive_state,
        keepalive_shutdown_rx,
    ));

    // Start the IPC listener
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);
    let ipc_state = Arc::clone(&state);
    let socket = args.socket_path.clone();
    let ipc_shutdown_tx = shutdown_tx.clone();
    tokio::spawn(async move {
        ipc::start_ipc_listener(&socket, ipc_state, ipc_shutdown_tx).await;
    });

    // Start the Veilid dispatch loop
    let dispatch_state = Arc::clone(&state);
    tokio::spawn(server_dispatch_loop(dispatch_state, update_rx));

    // Load previously hosted communities from DB
    load_persisted_communities(&state).await;

    // Spawn fast route recovery for communities that failed initial allocation
    let route_retry_state = Arc::clone(&state);
    tokio::spawn(retry_failed_routes(route_retry_state));

    tracing::info!(socket = %args.socket_path, "rekindle-server ready");

    // Wait for shutdown signal
    shutdown_rx.recv().await;

    tracing::info!("rekindle-server shutting down");

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
    let _ = std::fs::remove_file(&args.socket_path);

    tracing::info!("rekindle-server stopped");
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
                    if let Err(e) = mgr
                        .set_value(dht_key, SUBKEY_SERVER_ROUTE, rb.blob)
                        .await
                    {
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

/// Start the Veilid node with server-specific configuration.
///
/// Uses `VeilidConfig::new()` to generate a complete config with all required
/// fields (including any added in future veilid-core versions), then passes it
/// to `api_startup`. This is the same approach the client uses via
/// `RekindleNode::start()` — avoids the fragile hand-crafted JSON that breaks
/// when veilid-core adds new required config fields.
async fn start_veilid_node(
    storage_dir: &str,
    update_tx: mpsc::Sender<VeilidUpdate>,
) -> Result<veilid_core::VeilidAPI, String> {
    let update_callback: veilid_core::UpdateCallback = Arc::new(move |update: VeilidUpdate| {
        // Non-blocking send — if the channel is full we drop the event
        if let Err(e) = update_tx.try_send(update) {
            let dropped = match &e {
                mpsc::error::TrySendError::Full(u)
                | mpsc::error::TrySendError::Closed(u) => u,
            };
            tracing::error!(
                event = server_update_name(dropped),
                "Veilid update channel full — dropped event"
            );
        }
    });

    // VeilidConfig::new() generates a complete config with all defaults,
    // including any newly added fields like `consensus_width`. The storage_dir
    // override ensures the server uses its own separate storage from the client.
    let veilid_config = veilid_core::VeilidConfig::new(
        "rekindle-server",          // program_name
        "com",                      // organization
        "rekindle-server",          // qualifier (different from client to avoid collisions)
        Some(storage_dir),          // storage_directory override
        None,                       // config_directory (use default)
    );

    let api = veilid_core::api_startup(update_callback, veilid_config)
        .await
        .map_err(|e| format!("veilid api_startup failed: {e}"))?;

    api.attach().await.map_err(|e| format!("veilid attach failed: {e}"))?;

    tracing::info!("Veilid node started for server");
    Ok(api)
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
                    ).await;
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

    for (id, dht_key, keypair_hex, name, creator_pseudonym) in communities {
        // Pass the stored creator_pseudonym — host_community will skip
        // re-registering if the creator is already in the members table.
        if let Err(e) =
            community_host::host_community(state, &id, &dht_key, &keypair_hex, &name, &creator_pseudonym, "").await
        {
            tracing::error!(community = %id, error = %e, "failed to re-host community");
        }
    }
}

/// Load persisted community records from the server database.
#[allow(clippy::type_complexity)]
fn load_communities_from_db(state: &Arc<ServerState>) -> Result<Vec<(String, String, String, String, String)>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let mut stmt = db
        .prepare("SELECT id, dht_record_key, owner_keypair_hex, name, creator_pseudonym FROM hosted_communities")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

fn timestamp_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
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

/// Return a human-readable name for a `VeilidUpdate` variant (for logging).
fn server_update_name(update: &VeilidUpdate) -> &'static str {
    match update {
        VeilidUpdate::AppCall(_) => "AppCall",
        VeilidUpdate::AppMessage(_) => "AppMessage",
        VeilidUpdate::RouteChange(_) => "RouteChange",
        VeilidUpdate::Attachment(_) => "Attachment",
        VeilidUpdate::ValueChange(_) => "ValueChange",
        VeilidUpdate::Shutdown => "Shutdown",
        _ => "Other",
    }
}

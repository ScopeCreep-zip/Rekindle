//! IPC bus server — accepts connections, performs Noise IK handshakes,
//! dispatches daemon-bound requests, and routes agent-to-agent messages.
//!
//! The bus server lives inside the rekindle-node daemon process. It binds
//! a `UnixListener`, accepts client connections with `UCred` authentication,
//! performs encrypted handshakes, and either:
//! - Dispatches `IpcRequest` frames to `daemon::dispatch` and returns the
//!   `IpcResponse` to the originating connection (request-response pattern)
//! - Routes non-IpcRequest frames between connected agents (pub-sub pattern)

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, RwLock};
use uuid::Uuid;

use super::error::{IpcError, Result};
use super::framing::{decode_frame, encode_frame};
use super::message::{Message, SecurityLevel};
use super::noise::{self, NoiseTransport};
use super::protocol::BusPayload;
use super::registry::ClearanceRegistry;
use super::transport::{extract_ucred, PeerCredentials};

/// Rate limit: max requests per second per connection.
const RATE_LIMIT_MAX_TOKENS: u32 = 100;
/// Rate limit: refill interval in milliseconds.
const RATE_LIMIT_REFILL_MS: u64 = 1000;

/// Per-connection state tracked by the bus server.
struct ConnectionState {
    /// Agent identity (set on first message, immutable thereafter).
    agent_id: Option<Uuid>,
    /// Registry-verified agent name from Noise IK handshake.
    verified_name: Option<String>,
    /// Outbound channel to this connection's I/O task.
    tx: mpsc::Sender<Vec<u8>>,
    /// Peer OS-level credentials.
    peer: PeerCredentials,
    /// Security clearance level from registry lookup.
    security_clearance: SecurityLevel,
    /// When this connection was established.
    connected_at: Instant,
    /// Token bucket rate limiter: tokens remaining this window.
    rate_tokens: std::sync::atomic::AtomicU32,
    /// Epoch ms when tokens were last refilled.
    last_token_refill: std::sync::atomic::AtomicU64,
}

/// Shared state for the bus server, accessible from per-connection tasks.
struct ServerState {
    /// Active connections.
    connections: RwLock<HashMap<u64, ConnectionState>>,
    /// Request-response routing: msg_id → originating connection_id.
    pending_requests: RwLock<HashMap<Uuid, u64>>,
    /// Name-based unicast: verified_name → connection_id.
    name_to_conn: RwLock<HashMap<String, u64>>,
    /// Connection ID generator.
    next_conn_id: AtomicU64,
    /// Monotonic epoch for timestamps.
    epoch: Instant,
    /// Agent identity and clearance registry.
    registry: RwLock<ClearanceRegistry>,
    /// Inverted-index event router for O(1) per-event subscription delivery.
    event_router: parking_lot::RwLock<crate::daemon::event_router::EventRouter>,
}

/// The IPC bus server.
pub struct BusServer {
    listener: UnixListener,
    socket_path: PathBuf,
    state: Arc<ServerState>,
    /// Noise IK static keypair for the server.
    keypair: Arc<snow::Keypair>,
}

impl BusServer {
    /// Bind the bus server to a Unix domain socket.
    ///
    /// Creates parent directory if needed. Removes stale socket before binding.
    /// Sets directory permissions to 0700 and socket to 0600. [RC-4][RC-6]
    pub fn bind(path: &Path, keypair: snow::Keypair, registry: ClearanceRegistry) -> Result<Self> {
        // Create parent directory.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| IpcError::DirectoryCreate {
                path: parent.display().to_string(),
                source: e,
            })?;

            // [RC-6] Restrict parent directory to owner-only.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).map_err(
                    |e| IpcError::DirectoryCreate {
                        path: parent.display().to_string(),
                        source: e,
                    },
                )?;
            }
        }

        // Remove stale socket. Ignore ENOENT — no TOCTOU race. [RC-4]
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(IpcError::SocketBind {
                    path: path.display().to_string(),
                    source: e,
                });
            }
        }

        let listener = UnixListener::bind(path).map_err(|e| IpcError::SocketBind {
            path: path.display().to_string(),
            source: e,
        })?;

        // [RC-6] Restrict socket permissions.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(
                |e| IpcError::SocketBind {
                    path: path.display().to_string(),
                    source: e,
                },
            )?;
        }

        tracing::info!(path = %path.display(), "IPC bus server bound");

        Ok(Self {
            listener,
            socket_path: path.to_owned(),
            state: Arc::new(ServerState {
                connections: RwLock::new(HashMap::new()),
                pending_requests: RwLock::new(HashMap::new()),
                name_to_conn: RwLock::new(HashMap::new()),
                next_conn_id: AtomicU64::new(1),
                epoch: Instant::now(),
                registry: RwLock::new(registry),
                event_router: parking_lot::RwLock::new(
                    crate::daemon::event_router::EventRouter::new(),
                ),
            }),
            keypair: Arc::new(keypair),
        })
    }

    /// Run the accept loop. This future runs until cancelled.
    ///
    /// Each accepted connection spawns a per-connection task that handles
    /// the Noise handshake and bidirectional encrypted I/O.
    pub async fn run(&self) -> Result<()> {
        loop {
            match self.listener.accept().await {
                Ok((stream, _addr)) => {
                    let peer = match extract_ucred(&stream) {
                        Ok(creds) => creds,
                        Err(e) => {
                            tracing::error!(error = %e, "rejecting: UCred extraction failed");
                            continue;
                        }
                    };

                    // [RC-6] Same-UID enforcement.
                    let my_uid = PeerCredentials::local().uid;
                    if peer.uid != my_uid {
                        tracing::error!(peer_uid = peer.uid, my_uid, "rejecting: UID mismatch");
                        continue;
                    }

                    let conn_id = self.state.next_conn_id.fetch_add(1, Ordering::Relaxed);
                    tracing::info!(conn_id, pid = peer.pid, "client connected");

                    let (tx, rx) = mpsc::channel::<Vec<u8>>(256);
                    let state = Arc::clone(&self.state);
                    let keypair = Arc::clone(&self.keypair);

                    tokio::spawn(async move {
                        handle_connection(state, conn_id, stream, tx, rx, peer, keypair).await;
                    });
                }
                Err(e) => {
                    // [RC-1] Log actual OS error, don't conflate.
                    tracing::error!(error = %e, "accept failed");
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            }
        }
    }

    /// Access the registry for mutation (agent registration, rotation).
    pub async fn registry_mut(&self) -> tokio::sync::RwLockWriteGuard<'_, ClearanceRegistry> {
        self.state.registry.write().await
    }

    /// Number of active connections.
    pub async fn connection_count(&self) -> usize {
        self.state.connections.read().await.len()
    }

    /// The server's monotonic epoch.
    pub fn epoch(&self) -> Instant {
        self.state.epoch
    }

    /// The socket path this server is bound to.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Start the event delivery system using a watch channel.
    ///
    /// The watch receiver notifies this server when a subscription event source
    /// becomes available (on unlock) or unavailable (on lock). The delivery task
    /// subscribes to the broadcast channel and routes events through the
    /// EventRouter to subscribed connections.
    ///
    /// Re-lock safe: when the sender is replaced, the old broadcast closes,
    /// the delivery loop breaks, and the task re-awaits the next `.changed()`.
    pub fn start_event_delivery(
        &self,
        mut event_watch_rx: tokio::sync::watch::Receiver<
            Option<
                tokio::sync::broadcast::Sender<
                    rekindle_types::subscription_events::SubscriptionEvent,
                >,
            >,
        >,
    ) {
        let state = Arc::clone(&self.state);
        tokio::spawn(async move {
            loop {
                // Wait for the event source to become available.
                if event_watch_rx.changed().await.is_err() {
                    // Watch sender dropped — daemon shutting down.
                    tracing::info!("event delivery: watch channel closed, exiting");
                    break;
                }

                let sender = {
                    let guard = event_watch_rx.borrow();
                    guard.clone()
                };

                let Some(sender) = sender else {
                    // Source cleared (lock transition) — loop back and wait.
                    continue;
                };

                // Subscribe and deliver until the broadcast closes.
                let mut rx = sender.subscribe();
                tracing::info!("event delivery task started");

                loop {
                    match rx.recv().await {
                        Ok(event) => {
                            let (delivered, dropped) = state.event_router.read().deliver(&event);
                            if dropped > 0 {
                                tracing::debug!(
                                    delivered,
                                    dropped,
                                    "event delivery: some recipients dropped"
                                );
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!(skipped = n, "event delivery: lagging, events dropped");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            tracing::info!("event delivery: broadcast closed (lock or shutdown)");
                            break;
                        }
                    }
                }
                // Broadcast closed — loop back to await next unlock.
            }
        });
    }
}

impl Drop for BusServer {
    fn drop(&mut self) {
        // Clean up socket file on shutdown.
        let _ = std::fs::remove_file(&self.socket_path);
        tracing::info!(path = %self.socket_path.display(), "IPC socket removed");
    }
}

/// Handle a single client connection: Noise handshake, then encrypted I/O.
///
/// Connection is registered in state AFTER handshake succeeds — no frames
/// can arrive on `tx` before the writer task is spawned. [RC-11]
async fn handle_connection(
    state: Arc<ServerState>,
    conn_id: u64,
    stream: UnixStream,
    tx: mpsc::Sender<Vec<u8>>,
    mut outbound_rx: mpsc::Receiver<Vec<u8>>,
    peer_creds: PeerCredentials,
    keypair: Arc<snow::Keypair>,
) {
    let (reader, writer) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(reader);
    let mut writer = tokio::io::BufWriter::new(writer);

    let local_creds = PeerCredentials::local();
    let connected_at = Instant::now();

    // Noise IK handshake
    let mut transport: NoiseTransport = match noise::server_handshake(
        &mut reader,
        &mut writer,
        &keypair,
        &local_creds,
        &peer_creds,
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(
                conn_id,
                peer_pid = peer_creds.pid,
                error = %e,
                "Noise handshake failed"
            );
            return;
        }
    };

    // Extract client's X25519 static pubkey.
    let Some(key) = transport.remote_static() else {
        tracing::error!(conn_id, "no remote static key after handshake");
        return;
    };
    let Ok(client_pubkey): std::result::Result<[u8; 32], _> = key.try_into() else {
        tracing::error!(conn_id, "client pubkey not 32 bytes");
        return;
    };

    // Registry lookup: pubkey → (name, clearance).
    let (security_clearance, verified_name) = {
        let reg = state.registry.read().await;
        if let Some(identity) = reg.lookup(&client_pubkey) {
            let name = reg.lookup_name(&client_pubkey).map(str::to_owned);
            tracing::info!(
                conn_id,
                agent = name.as_deref().unwrap_or("unknown"),
                clearance = ?identity.security_level,
                "agent authenticated via registry"
            );
            (identity.security_level, name)
        } else {
            tracing::info!(
                conn_id,
                clearance = ?SecurityLevel::Open,
                "ephemeral client accepted (not in registry)"
            );
            (SecurityLevel::Open, None)
        }
    };

    // Register connection AFTER handshake.
    let conn = ConnectionState {
        agent_id: None,
        verified_name: verified_name.clone(),
        tx,
        peer: peer_creds,
        security_clearance,
        connected_at,
        rate_tokens: std::sync::atomic::AtomicU32::new(RATE_LIMIT_MAX_TOKENS),
        last_token_refill: std::sync::atomic::AtomicU64::new(0),
    };
    state.connections.write().await.insert(conn_id, conn);

    // Populate name_to_conn for unicast routing.
    if let Some(ref name) = verified_name {
        state
            .name_to_conn
            .write()
            .await
            .insert(name.clone(), conn_id);
    }

    // Multiplexed I/O loop. [RC-16] All buffers zeroized after use.
    loop {
        tokio::select! {
            result = transport.read_encrypted_frame(&mut reader) => {
                match result {
                    Ok(mut payload) => {
                        route_frame(&state, conn_id, &payload).await;
                        zeroize::Zeroize::zeroize(&mut payload);
                    }
                    Err(e) => {
                        let session_ms = connected_at.elapsed().as_millis();
                        tracing::info!(
                            conn_id,
                            session_ms = %session_ms,
                            error = %e,
                            "client disconnected"
                        );
                        break;
                    }
                }
            }
            Some(mut payload) = outbound_rx.recv() => {
                let result = transport.write_encrypted_frame(&mut writer, &payload).await;
                zeroize::Zeroize::zeroize(&mut payload);
                if let Err(e) = result {
                    tracing::debug!(conn_id, error = %e, "write failed, closing");
                    break;
                }
            }
            else => break,
        }
    }

    // Cleanup: remove from all routing tables and broadcast disconnect event.
    let disconnected_name = {
        let conns = state.connections.read().await;
        match conns.get(&conn_id) {
            Some(c) => {
                #[allow(clippy::cast_possible_truncation)]
                let duration = c.connected_at.elapsed().as_millis() as u64;
                tracing::info!(
                    conn_id,
                    agent = c.verified_name.as_deref().unwrap_or("ephemeral"),
                    peer_pid = c.peer.pid,
                    session_ms = duration,
                    "connection cleanup"
                );
                c.verified_name.clone()
            }
            None => None,
        }
    };
    state.connections.write().await.remove(&conn_id);
    state.event_router.write().remove_connection(conn_id);
    state
        .pending_requests
        .write()
        .await
        .retain(|_, cid| *cid != conn_id);
    state
        .name_to_conn
        .write()
        .await
        .retain(|_, cid| *cid != conn_id);

    // Log agent disconnect. Event delivery to subscribers is handled by the
    // subscription manager, not by broadcasting raw frames on the bus.
    if let Some(ref name) = disconnected_name {
        tracing::info!(
            agent = %name,
            conn_id,
            "agent disconnected"
        );
    }
}

/// Route a received frame to the appropriate destination(s).
///
/// Pure router — the server has zero knowledge of IPC request/response
/// semantics. It decodes the `Message<BusPayload>` envelope for routing
/// metadata (correlation_id, sender, security_level, verified_sender_name)
/// and forwards the frame to the correct destination(s).
///
/// - If `correlation_id` is set: this is a response — route to the
///   connection that originated the matching request.
/// - If `correlation_id` is None: this is a new request or broadcast —
///   record the `msg_id` for response routing and forward to all other
///   connections at sufficient clearance.
async fn route_frame(state: &ServerState, sender_conn_id: u64, payload: &[u8]) {
    // Decode the message envelope. Single type for all bus traffic.
    let mut msg: Message<BusPayload> = match decode_frame(payload) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(conn_id = sender_conn_id, error = %e, "malformed frame, dropping");
            return;
        }
    };

    // Rate limit check: token bucket per connection.
    {
        let conns = state.connections.read().await;
        if let Some(conn) = conns.get(&sender_conn_id) {
            let now_ms = rekindle_utils::timestamp_ms();
            let last = conn
                .last_token_refill
                .load(std::sync::atomic::Ordering::Relaxed);
            if now_ms.saturating_sub(last) >= RATE_LIMIT_REFILL_MS {
                conn.rate_tokens
                    .store(RATE_LIMIT_MAX_TOKENS, std::sync::atomic::Ordering::Relaxed);
                conn.last_token_refill
                    .store(now_ms, std::sync::atomic::Ordering::Relaxed);
            }
            let prev = conn.rate_tokens.fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |t| if t > 0 { Some(t - 1) } else { None },
            );
            if prev.is_err() {
                tracing::warn!(
                    conn_id = sender_conn_id,
                    "rate limit exceeded, dropping frame"
                );
                return;
            }
        }
    }

    // Sender identity verification and clearance enforcement.
    {
        let mut conns = state.connections.write().await;
        if let Some(conn) = conns.get_mut(&sender_conn_id) {
            // Identity immutability: once set, agent_id cannot change mid-session.
            if let Some(known_id) = conn.agent_id {
                if known_id != msg.sender {
                    tracing::warn!(
                        conn_id = sender_conn_id,
                        expected = %known_id,
                        got = %msg.sender,
                        "agent identity changed mid-session, dropping frame"
                    );
                    return;
                }
            } else {
                conn.agent_id = Some(msg.sender);
            }

            // [RC-6] Sender clearance enforcement.
            if conn.security_clearance < msg.security_level {
                tracing::warn!(
                    conn_id = sender_conn_id,
                    sender = ?conn.security_clearance,
                    msg = ?msg.security_level,
                    "clearance insufficient, rejecting"
                );
                return;
            }
        }
    }

    // Stamp verified_sender_name from connection state.
    {
        let conns = state.connections.read().await;
        if let Some(conn) = conns.get(&sender_conn_id) {
            msg.verified_sender_name.clone_from(&conn.verified_name);
        }
    }

    // Re-encode with server stamps applied.
    let stamped_payload = match encode_frame(&msg) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(conn_id = sender_conn_id, error = %e, "re-encode failed");
            return;
        }
    };

    // ── Fail-closed routing ──────────────────────────────────────────
    //
    // Messages are NEVER broadcast to all connections. Every message is
    // either a correlated response (unicast to originator), a request
    // (unicast to the daemon), or an event (subscription-filtered).
    // Anything that doesn't match a known routing path is dropped.

    if let Some(corr_id) = msg.correlation_id {
        // Correlated response: route to the connection that sent the original request.
        let target_conn = state.pending_requests.write().await.remove(&corr_id);
        if let Some(target_id) = target_conn {
            let conns = state.connections.read().await;
            if let Some(target) = conns.get(&target_id) {
                if target.tx.try_send(stamped_payload).is_err() {
                    tracing::warn!(conn_id = target_id, "response dropped: channel full");
                }
            }
        } else {
            tracing::debug!(correlation_id = %corr_id, "response for unknown request, dropping");
        }
        return;
    }

    // No correlation_id — route based on payload type.
    match &msg.payload {
        BusPayload::Request(ref request) => {
            // Intercept Subscribe/Unsubscribe — handled server-side via EventRouter.
            // These never reach the daemon dispatch.
            match request {
                super::protocol::IpcRequest::Subscribe { filters } => {
                    let sender_tx = {
                        let conns = state.connections.read().await;
                        conns.get(&sender_conn_id).map(|c| c.tx.clone())
                    };
                    let response = if let Some(tx) = sender_tx {
                        match state
                            .event_router
                            .write()
                            .subscribe(sender_conn_id, filters, tx)
                        {
                            Ok(count) => super::protocol::IpcResponse::ok(&serde_json::json!({
                                "subscribed": true, "filter_count": count,
                            })),
                            Err(reason) => super::protocol::IpcResponse::error(400, reason),
                        }
                    } else {
                        super::protocol::IpcResponse::error(500, "connection state not found")
                    };
                    // Send response directly back to sender
                    let resp_msg = Message {
                        wire_version: super::message::WIRE_VERSION,
                        msg_id: msg.msg_id,
                        sender: Uuid::nil(),
                        correlation_id: Some(msg.msg_id),
                        timestamp: super::message::Timestamp::now(state.epoch),
                        security_level: msg.security_level,
                        verified_sender_name: Some(DAEMON_AGENT_NAME.to_string()),
                        agent_type: None,
                        community_scope: None,
                        payload: BusPayload::Response(
                            serde_json::to_vec(&response).unwrap_or_else(|_| b"{}".to_vec()),
                        ),
                    };
                    if let Ok(bytes) = encode_frame(&resp_msg) {
                        let conns = state.connections.read().await;
                        if let Some(conn) = conns.get(&sender_conn_id) {
                            let _ = conn.tx.try_send(bytes);
                        }
                    }
                }
                super::protocol::IpcRequest::Unsubscribe { filters } => {
                    let remaining = state
                        .event_router
                        .write()
                        .unsubscribe(sender_conn_id, filters);
                    let response = super::protocol::IpcResponse::ok(&serde_json::json!({
                        "unsubscribed": true, "remaining": remaining,
                    }));
                    let resp_msg = Message {
                        wire_version: super::message::WIRE_VERSION,
                        msg_id: msg.msg_id,
                        sender: Uuid::nil(),
                        correlation_id: Some(msg.msg_id),
                        timestamp: super::message::Timestamp::now(state.epoch),
                        security_level: msg.security_level,
                        verified_sender_name: Some(DAEMON_AGENT_NAME.to_string()),
                        agent_type: None,
                        community_scope: None,
                        payload: BusPayload::Response(
                            serde_json::to_vec(&response).unwrap_or_else(|_| b"{}".to_vec()),
                        ),
                    };
                    if let Ok(bytes) = encode_frame(&resp_msg) {
                        let conns = state.connections.read().await;
                        if let Some(conn) = conns.get(&sender_conn_id) {
                            let _ = conn.tx.try_send(bytes);
                        }
                    }
                }
                _ => {
                    // All other requests: record for response routing, forward to daemon.
                    state
                        .pending_requests
                        .write()
                        .await
                        .insert(msg.msg_id, sender_conn_id);
                    let daemon_conn = state
                        .name_to_conn
                        .read()
                        .await
                        .get(DAEMON_AGENT_NAME)
                        .copied();
                    if let Some(daemon_id) = daemon_conn {
                        let conns = state.connections.read().await;
                        if let Some(daemon) = conns.get(&daemon_id) {
                            if daemon.tx.try_send(stamped_payload).is_err() {
                                tracing::warn!("request dropped: daemon channel full");
                            }
                        }
                    } else {
                        tracing::error!("request dropped: daemon not connected to bus");
                    }
                }
            }
        }
        BusPayload::Response(_) => {
            tracing::warn!(
                conn_id = sender_conn_id,
                "uncorrelated response without correlation_id, dropping"
            );
        }
        BusPayload::Event(ref event) => {
            // Events from the daemon's bridge task: route via EventRouter.
            // Only the daemon agent may send events. Other sources are rejected.
            let is_daemon = {
                let conns = state.connections.read().await;
                conns
                    .get(&sender_conn_id)
                    .and_then(|c| c.verified_name.as_deref())
                    == Some(DAEMON_AGENT_NAME)
            };
            if !is_daemon {
                tracing::warn!(
                    conn_id = sender_conn_id,
                    "event from non-daemon source — rejected"
                );
                return;
            }
            let (delivered, dropped) = state.event_router.read().deliver(event);
            tracing::debug!(delivered, dropped, "event routed via EventRouter");
        }
    }
}

/// Well-known agent name for the daemon's internal bus subscriber.
///
/// The daemon registers with this name when it connects to its own socket.
/// Requests are unicast to this connection. Other agents MUST NOT register
/// with this name — the registry enforces uniqueness.
pub const DAEMON_AGENT_NAME: &str = "daemon";

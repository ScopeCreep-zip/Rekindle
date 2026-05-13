//! Shared state types for the IPC bus server.
//!
//! All types in this module are data structures with no async code.
//! They are shared across connection handler tasks via `Arc<ServerState>`.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

use bytes::Bytes;
use dashmap::DashMap;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::ipc::event_router::ShardedEventRouter;
use crate::ipc::message::{SecurityLevel, SharedFrame, RoutedFrame};
use crate::ipc::registry::ClearanceRegistry;
use crate::ipc::transport::PeerCredentials;

use super::constants::{RATE_LIMIT_MAX_TOKENS, RATE_LIMIT_REFILL_MS};

// ── Token Bucket Rate Limiter ─────────────────────────────────────────

/// Lock-free per-connection rate limiter using atomic operations.
///
/// Approximate rate limiting: the refill is not atomic with respect to
/// consumption. Both the CAS on `last_refill_ms` and the `store` to
/// `tokens` are separate operations. This is acceptable — the invariant
/// is "approximately N requests per window", not "exactly N".
pub struct TokenBucket {
    tokens: AtomicU32,
    last_refill_ms: AtomicU64,
}

impl Default for TokenBucket {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenBucket {
    pub fn new() -> Self {
        Self {
            tokens: AtomicU32::new(RATE_LIMIT_MAX_TOKENS),
            last_refill_ms: AtomicU64::new(0),
        }
    }

    /// Try to consume one token. Returns true if allowed.
    pub fn try_consume(&self) -> bool {
        let now_ms = wall_clock_ms();
        let last = self.last_refill_ms.load(Ordering::Relaxed);
        if now_ms.saturating_sub(last) >= RATE_LIMIT_REFILL_MS
            && self
                .last_refill_ms
                .compare_exchange_weak(last, now_ms, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            self.tokens.store(RATE_LIMIT_MAX_TOKENS, Ordering::Relaxed);
        }

        self.tokens
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |t| {
                if t > 0 { Some(t - 1) } else { None }
            })
            .is_ok()
    }
}

#[inline]
fn wall_clock_ms() -> u64 {
    #[allow(clippy::cast_possible_truncation)]
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ── Per-Connection State ──────────────────────────────────────────────

/// Per-connection state tracked by the bus server.
pub struct ConnectionState {
    /// Agent identity. Set-once on first message via `OnceLock`.
    pub agent_id: OnceLock<Uuid>,
    /// Registry-verified agent name. Immutable after handshake.
    pub verified_name: Option<Arc<str>>,
    /// Response channel: daemon responses + server-generated responses.
    pub response_tx: mpsc::Sender<Bytes>,
    /// Event channel: subscription events from EventRouter fan-out.
    pub event_tx: mpsc::Sender<SharedFrame>,
    /// Peer OS-level credentials. Immutable after handshake.
    pub peer: PeerCredentials,
    /// Security clearance level. Immutable after handshake.
    pub security_clearance: SecurityLevel,
    /// When this connection was established.
    pub connected_at: Instant,
    /// Lock-free rate limiter.
    pub rate_limiter: TokenBucket,
    /// Nonce counter from the connection's BulkSession. Set after
    /// bulk session construction. Read by cancel handler.
    pub bulk_nonce_counter: Option<Arc<std::sync::atomic::AtomicU64>>,
    /// Cancel signal channel for reassembler reset.
    pub cancel_bulk_tx: mpsc::Sender<u64>,
}

// ── PendingRequests Dual Index ────────────────────────────────────────

/// Dual-indexed request-response correlation.
///
/// Primary: `by_msg_id` — O(1) lookup on response arrival.
/// Secondary: `by_conn_id` — O(pending_for_conn) cleanup on disconnect.
pub struct PendingRequests {
    by_msg_id: HashMap<Uuid, u64>,
    by_conn_id: HashMap<u64, smallvec::SmallVec<[Uuid; 4]>>,
}

impl Default for PendingRequests {
    fn default() -> Self {
        Self::new()
    }
}

impl PendingRequests {
    pub fn new() -> Self {
        Self {
            by_msg_id: HashMap::new(),
            by_conn_id: HashMap::new(),
        }
    }

    pub fn insert(&mut self, msg_id: Uuid, conn_id: u64) {
        self.by_msg_id.insert(msg_id, conn_id);
        self.by_conn_id.entry(conn_id).or_default().push(msg_id);
    }

    pub fn remove_by_msg_id(&mut self, msg_id: &Uuid) -> Option<u64> {
        let conn_id = self.by_msg_id.remove(msg_id)?;
        if let Some(set) = self.by_conn_id.get_mut(&conn_id) {
            set.retain(|id| id != msg_id);
            if set.is_empty() {
                self.by_conn_id.remove(&conn_id);
            }
        }
        Some(conn_id)
    }

    pub fn remove_by_conn_id(&mut self, conn_id: u64) {
        if let Some(msg_ids) = self.by_conn_id.remove(&conn_id) {
            for id in msg_ids {
                self.by_msg_id.remove(&id);
            }
        }
    }
}

// ── Server State ──────────────────────────────────────────────────────

/// Shared state for the bus server, accessible from per-connection tasks.
///
/// Lock types chosen for each field's access pattern:
/// - `DashMap`: sharded, high concurrency reads+writes (connections)
/// - `parking_lot::Mutex`: uncontended single-writer (pending_requests)
/// - `parking_lot::RwLock`: read-heavy, rare writes (name_to_conn, registry)
/// - `Arc`: interior locking (event_router)
impl ServerState {
    /// Signal a connection's reassembler to reset after bulk transfer cancel.
    pub fn cancel_bulk_stream(&self, conn_id: u64, next_nonce: u64) {
        if let Some(conn) = self.connections.get(&conn_id) {
            let _ = conn.cancel_bulk_tx.try_send(next_nonce);
        }
    }
}

pub struct ServerState {
    pub connections: DashMap<u64, ConnectionState>,
    pub pending_requests: parking_lot::Mutex<PendingRequests>,
    pub name_to_conn: parking_lot::RwLock<HashMap<String, u64>>,
    pub next_conn_id: AtomicU64,
    pub epoch: Instant,
    pub registry: parking_lot::RwLock<ClearanceRegistry>,
    pub event_router: Arc<ShardedEventRouter>,
    /// Direct channel to the daemon subscriber for structured request forwarding.
    pub daemon_tx: parking_lot::RwLock<Option<mpsc::Sender<RoutedFrame>>>,
}

static_assertions::assert_impl_all!(ServerState: Send, Sync);

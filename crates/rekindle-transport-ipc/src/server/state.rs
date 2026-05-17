//! Shared state types for the IPC bus server.
//!
//! All types are data structures — no async code. Shared across
//! connection handler tasks via Arc<ServerState>.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use dashmap::DashMap;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::envelope::{SecurityLevel, SharedFrame};
use crate::socket::PeerCredentials;
use crate::transport_frame::{ConnectionPhase, BulkOutcome};

/// Lock-free per-connection rate limiter. Approximate token bucket.
pub struct TokenBucket {
    tokens: AtomicU32,
    last_refill_ms: AtomicU64,
    max_tokens: u32,
    refill_ms: u64,
}

impl TokenBucket {
    pub fn new(max_tokens: u32, refill_ms: u64) -> Self {
        Self {
            tokens: AtomicU32::new(max_tokens),
            last_refill_ms: AtomicU64::new(0),
            max_tokens,
            refill_ms,
        }
    }

    pub fn try_consume(&self) -> bool {
        let now_ms = wall_clock_ms();
        let last = self.last_refill_ms.load(Ordering::Relaxed);
        if now_ms.saturating_sub(last) >= self.refill_ms
            && self
                .last_refill_ms
                .compare_exchange_weak(last, now_ms, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            self.tokens.store(self.max_tokens, Ordering::Relaxed);
        }
        self.tokens
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |t| {
                if t > 0 { Some(t - 1) } else { None }
            })
            .is_ok()
    }
}

impl Default for TokenBucket {
    fn default() -> Self {
        Self::new(100, 1000)
    }
}

#[allow(clippy::cast_possible_truncation)]
#[inline]
fn wall_clock_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Per-connection ack tracking entry.
/// The sender registers a oneshot for each seq; the receiver's Ack resolves it.
pub struct PendingAck {
    pub tx: oneshot::Sender<()>,
}

/// Per-connection bulk transfer tracking.
pub struct PendingBulk {
    pub tx: oneshot::Sender<BulkOutcome>,
    pub chunks_sent: u64,
    pub bytes_sent: u64,
    pub started_at: Instant,
}

/// Per-connection state tracked by the bus server.
pub struct ConnectionState {
    /// Agent identity. Set-once on first message.
    pub agent_id: std::sync::OnceLock<Uuid>,
    /// Registry-verified agent name. Immutable after handshake.
    pub verified_name: Option<Arc<str>>,
    /// Response channel (application-level correlated responses).
    pub response_tx: mpsc::Sender<Bytes>,
    /// Event channel for subscription push.
    pub event_tx: mpsc::Sender<SharedFrame>,
    /// Peer OS-level credentials.
    pub peer: PeerCredentials,
    /// Security clearance level.
    pub security_clearance: SecurityLevel,
    /// Connection establishment time.
    pub connected_at: Instant,
    /// Per-connection rate limiter.
    pub rate_limiter: TokenBucket,
    /// Nonce counter from bulk session (for cancel handler).
    pub bulk_nonce_counter: Option<Arc<crate::bulk::NonceCounter>>,
    /// Cancel signal: carries stream_id to cancel a specific stream's reassembler.
    pub cancel_bulk_tx: mpsc::Sender<u8>,
    /// Current connection lifecycle phase.
    pub phase: parking_lot::Mutex<ConnectionPhase>,
    /// Pending ack waiters: seq -> oneshot.
    pub pending_acks: parking_lot::Mutex<HashMap<u64, PendingAck>>,
    /// Pending bulk transfer waiters: stream_id -> oneshot.
    pub pending_bulk: parking_lot::Mutex<HashMap<u8, PendingBulk>>,
    /// Channel for sending transport-internal frames (Ack, Heartbeat, etc.)
    /// to the write loop. Separate from response_tx to avoid priority inversion.
    pub transport_tx: mpsc::Sender<Vec<u8>>,
    /// Channel for sending bulk data frames to the write loop.
    /// Used for server-to-client bulk transfers.
    pub bulk_out_tx: mpsc::Sender<Vec<u8>>,
    /// Rayon pool for server-side bulk encryption (outbound transfers).
    pub encrypt_pool: Arc<rayon::ThreadPool>,
    /// Bulk cipher for server-to-client encryption. Same key as client
    /// (derived from shared handshake hash via HKDF).
    pub bulk_cipher: Option<Arc<crate::bulk::BulkCipher>>,
    /// Nonce counter for server-to-client bulk encryption.
    pub bulk_send_nonce: Arc<crate::bulk::NonceCounter>,
    /// Buffer pool for server-to-client bulk encryption.
    pub bulk_send_pool: Arc<crate::bulk::BufferPool>,
}

impl ConnectionState {
    /// Transition the connection phase. Returns Err if the transition is illegal.
    pub fn transition(&self, new: ConnectionPhase) -> Result<ConnectionPhase, ConnectionPhase> {
        let mut phase = self.phase.lock();
        let old = *phase;
        if old.can_transition_to(new) {
            *phase = new;
            Ok(old)
        } else {
            Err(old)
        }
    }

    /// Current phase.
    pub fn current_phase(&self) -> ConnectionPhase {
        *self.phase.lock()
    }

    /// Resolve a pending ack by seq number.
    pub fn resolve_ack(&self, seq: u64) {
        if let Some(pending) = self.pending_acks.lock().remove(&seq) {
            let _ = pending.tx.send(());
        }
    }

    /// Resolve a pending bulk transfer with an outcome.
    pub fn resolve_bulk(&self, stream_id: u8, outcome: BulkOutcome) {
        if let Some(pending) = self.pending_bulk.lock().remove(&stream_id) {
            let _ = pending.tx.send(outcome);
        }
    }

    /// Send a bulk payload to this client.
    ///
    /// Chunks the payload, encrypts each chunk via the rayon pool,
    /// and feeds encrypted frames through bulk_out_tx to the write loop.
    /// The write loop writes them to the socket on lane 0x01.
    ///
    /// This is fire-and-forget from the server's perspective — no ack
    /// tracking. The client's recv_bulk() delivers the reassembled payload.
    pub fn send_bulk(&self, stream_id: u8, payload: &[u8]) -> Result<(), &'static str> {
        let cipher = self.bulk_cipher.as_ref().ok_or("no bulk cipher")?;
        crate::bulk::transfer::send_payload(
            &self.encrypt_pool,
            cipher,
            &self.bulk_send_nonce,
            &self.bulk_send_pool,
            self.bulk_out_tx.clone(),
            stream_id,
            payload,
            crate::bulk::DigestAlgorithm::Blake3,
        );
        Ok(())
    }

    /// Resolve ALL pending acks and bulk transfers with connection-lost.
    pub fn resolve_all_pending_lost(&self) {
        let mut acks = self.pending_acks.lock();
        for (_, pending) in acks.drain() {
            let _ = pending.tx.send(());
        }
        let mut bulks = self.pending_bulk.lock();
        for (_, pending) in bulks.drain() {
            let _ = pending.tx.send(BulkOutcome::ConnectionLost);
        }
    }
}

/// Dual-indexed pending request tracker (application-level correlation).
pub struct PendingRequests {
    by_msg_id: HashMap<Uuid, u64>,
    by_conn_id: HashMap<u64, smallvec::SmallVec<[Uuid; 4]>>,
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

impl Default for PendingRequests {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared server state.
pub struct ServerState {
    pub connections: DashMap<u64, ConnectionState>,
    pub pending_requests: parking_lot::Mutex<PendingRequests>,
    pub name_to_conn: parking_lot::RwLock<HashMap<String, u64>>,
    pub next_conn_id: AtomicU64,
    pub epoch: Instant,
    /// Global rate limiter across all connections.
    pub global_rate_limiter: TokenBucket,
}

impl ServerState {
    /// Signal a connection to cancel a specific stream's reassembler.
    pub fn cancel_bulk_stream(&self, conn_id: u64, stream_id: u8) {
        if let Some(conn) = self.connections.get(&conn_id) {
            let _ = conn.cancel_bulk_tx.try_send(stream_id);
        }
    }
}

impl ServerState {
    /// Wait until exactly `n` connections are registered.
    /// Useful for health checks, graceful shutdown draining, and tests.
    /// Uses yield-based polling. Timeout panics with diagnostic message.
    pub async fn wait_connection_count(&self, n: usize, timeout: std::time::Duration) {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.connections.len() == n { return; }
            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "timed out waiting for {} connections, have {}",
                    n, self.connections.len()
                );
            }
            tokio::task::yield_now().await;
        }
    }
}

static_assertions::assert_impl_all!(ServerState: Send, Sync);

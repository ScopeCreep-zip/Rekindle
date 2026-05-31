//! W16.2 — `EnvelopeQueue` retry primitive.
//!
//! Reliable delivery of fire-and-forget envelopes (call signaling,
//! group call signaling, DM body, DM invites) over Veilid's `app_message`
//! transport. Handles persistence (via [`EnvelopeStore`]), per-recipient
//! sequencing (W16.3), retry with exponential backoff (matching Veilid's
//! 5-min private-route expiry), and an in-process oneshot registry for
//! the request-reply flavor (DM invites).
//!
//! # Architecture
//!
//! - **Payload-agnostic**: the queue accepts already-serialized bytes plus a
//!   [`TypeId`]. Caller does the serialization (e.g. postcard-encoding
//!   their `MessagePayload` variant). Keeps the queue independent of any
//!   specific payload schema; lets src-tauri serialize its `MessagePayload`
//!   while CLI/node could serialize their own types.
//!
//! - **Send dispatch**: routes through [`crate::Sender::send_dm`] which
//!   handles framing, signing (Ed25519), and the actual `app_message` call.
//!
//! - **Persistence**: every enqueued row survives crash via the configured
//!   [`EnvelopeStore`] impl (atomic JSON for CLI/node, SQLite for Tauri).
//!
//! - **Two send shapes**:
//!   - [`EnvelopeQueue::send`] — fire-and-forget. Returns immediately after
//!     persisting the row; the retry tick handles delivery.
//!   - [`EnvelopeQueue::send_expect_reply`] — same as `send` but registers a
//!     oneshot keyed by `correlation_id` and awaits a correlated reply
//!     envelope (delivered via [`EnvelopeQueue::deliver_reply`] from the
//!     `InboundHandler` impl). Times out independently of the retry cap.
//!
//! - **Notifications**: on delivery success or cap-hit, emits
//!   `TransportNotification::EnvelopeDelivered` /
//!   `EnvelopeDeliveryFailed` via [`crate::SharedState::notify`]. Each
//!   shell's subscriber bridges these to its own UI surface (Tauri
//!   `app.emit("chat-event", ...)`, CLI stdout, daemon IPC).
//!
//! See `.claude/plans/giggly-inventing-snowglobe.md` Wave 16 for the
//! full design.

pub mod retry_config;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rand::Rng;
use rekindle_types::notification::TransportNotification;
use tokio::sync::oneshot;
use tracing::{debug, warn};

use crate::broadcast::node::TransportNode;
use crate::envelope_store::{EnvelopeKind, EnvelopeStore, PendingEnvelope, StoreError};
use crate::error::TransportError;
use crate::frame::TypeId;
use crate::shared::SharedState;

pub use retry_config::RetryConfig;

/// How many envelopes a single retry tick processes at once. Bounds the
/// per-tick work; remaining rows pick up next tick (1 s cadence).
const TICK_BATCH_LIMIT: usize = 64;

/// Default `send_expect_reply` timeout — 60 s. DM invites are
/// user-tolerant; voice calls have their own ring timer at 30 s.
pub const DEFAULT_REPLY_TIMEOUT: Duration = Duration::from_secs(60);

/// Errors surfaced by the queue.
#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    /// The kind has no `TypeId` and can't be sent via `app_message`.
    /// (Currently only `FriendRequestInbox` / `FriendAcceptInbox`,
    /// which are DHT writes — W16.10 routes them through `operations::friend`.)
    #[error("envelope kind {0:?} has no wire TypeId; not deliverable via app_message")]
    UnsupportedKind(EnvelopeKind),

    /// Persistence layer error.
    #[error("store: {0}")]
    Store(StoreError),

    /// Transport send error (route lookup, app_message dispatch, etc.).
    #[error("transport: {0}")]
    Transport(TransportError),

    /// Reply timeout exceeded for an `expect_reply` send.
    #[error("reply timeout for correlation_id={0}")]
    ReplyTimeout(String),

    /// The reply oneshot was dropped before a value arrived (shouldn't happen
    /// in normal flow — would mean the queue was destroyed mid-call).
    #[error("reply receiver closed for correlation_id={0}")]
    ReplyDropped(String),
}

impl From<StoreError> for QueueError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

impl From<TransportError> for QueueError {
    fn from(e: TransportError) -> Self {
        Self::Transport(e)
    }
}

/// The reliability primitive. Cloned cheaply (internal `Arc`s); pass by
/// value into spawned tasks.
#[derive(Clone)]
pub struct EnvelopeQueue {
    inner: Arc<Inner>,
}

struct Inner {
    transport: Arc<TransportNode>,
    store: Arc<dyn EnvelopeStore>,
    notifications: Arc<SharedState>,
    /// Sender's Ed25519 secret. Stays in memory for the queue's lifetime;
    /// cleared by `Drop` (zeroize is a future hardening step).
    sender_secret: [u8; 32],
    sender_public_hex: String,
    /// Owner key for store scoping. Same as `sender_public_hex` in normal
    /// use; kept separate so multi-identity hosts can scope distinctly.
    owner_key: String,
    /// In-process oneshot registry for `expect_reply`. Keyed by
    /// `correlation_id`. Populated by `send_expect_reply`, drained by
    /// `deliver_reply`.
    pending_replies: Mutex<HashMap<String, oneshot::Sender<Vec<u8>>>>,
}

impl EnvelopeQueue {
    /// Construct a new queue scoped to a single sender identity.
    ///
    /// `sender_secret` is the Ed25519 signing key. `sender_public_hex` is
    /// the matching public key as lowercase hex (matches what
    /// [`crate::Sender::send_dm`] expects). `owner_key` is the store scope —
    /// same as `sender_public_hex` for single-identity hosts.
    pub fn new(
        transport: Arc<TransportNode>,
        store: Arc<dyn EnvelopeStore>,
        notifications: Arc<SharedState>,
        sender_secret: [u8; 32],
        sender_public_hex: impl Into<String>,
        owner_key: impl Into<String>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                transport,
                store,
                notifications,
                sender_secret,
                sender_public_hex: sender_public_hex.into(),
                owner_key: owner_key.into(),
                pending_replies: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Persist an envelope and return immediately. The retry tick
    /// dispatches it on the next 1 s pass.
    ///
    /// Use [`Self::send_expect_reply`] for request-response flows
    /// (DM invite, etc.).
    pub async fn send(
        &self,
        recipient: &str,
        payload_bytes: Vec<u8>,
        kind: EnvelopeKind,
        correlation_id: Option<&str>,
    ) -> std::result::Result<(), QueueError> {
        // Validate kind has a wire TypeId before persisting.
        if kind.wire_type_id().is_none() {
            return Err(QueueError::UnsupportedKind(kind));
        }

        let cfg = RetryConfig::for_kind(kind);
        let now_ms = rekindle_utils::timestamp_ms();

        // Allocate a per-recipient seq (W16.3 — receiver dedup keys on it).
        let seq = self
            .inner
            .store
            .next_outbound_seq(
                &self.inner.owner_key,
                recipient,
                kind,
                correlation_id.unwrap_or(""),
            )
            .await?;

        let env = PendingEnvelope {
            id: 0, // assigned by store
            owner_key: self.inner.owner_key.clone(),
            recipient_key: recipient.to_string(),
            kind,
            seq,
            correlation_id: correlation_id.map(str::to_string),
            payload: payload_bytes,
            created_at_ms: now_ms,
            next_retry_at_ms: now_ms,
            retry_count: 0,
            max_retries: cfg.max_retries,
            last_error: None,
        };

        self.inner.store.enqueue(env).await?;
        Ok(())
    }

    /// Persist an envelope and wait for a correlated reply or timeout.
    ///
    /// Used for DM invites and other request-reply flows. The receiver
    /// must call [`Self::deliver_reply`] with the matching
    /// `correlation_id` when the reply envelope arrives via the
    /// inbound dispatch.
    ///
    /// Returns the raw reply payload bytes — caller deserializes.
    /// Times out after `timeout` (use [`DEFAULT_REPLY_TIMEOUT`] for
    /// the standard 60 s).
    pub async fn send_expect_reply(
        &self,
        recipient: &str,
        payload_bytes: Vec<u8>,
        kind: EnvelopeKind,
        correlation_id: &str,
        timeout: Duration,
    ) -> std::result::Result<Vec<u8>, QueueError> {
        if !kind.is_request() {
            warn!(
                ?kind,
                "send_expect_reply called with a non-request kind; behavior degenerates to fire-and-forget"
            );
        }

        // Register the oneshot BEFORE persisting/sending. If the reply
        // arrives faster than this function returns (in-process race),
        // it has somewhere to land.
        let (tx, rx) = oneshot::channel::<Vec<u8>>();
        {
            let mut registry = self.inner.pending_replies.lock();
            registry.insert(correlation_id.to_string(), tx);
        }

        // Send the request via the same persistence path.
        if let Err(e) = self
            .send(recipient, payload_bytes, kind, Some(correlation_id))
            .await
        {
            // Clean up the oneshot on send failure.
            self.inner.pending_replies.lock().remove(correlation_id);
            return Err(e);
        }

        // Await the reply or time out.
        let result = tokio::time::timeout(timeout, rx).await;

        // Clean up the registry entry regardless of outcome.
        self.inner.pending_replies.lock().remove(correlation_id);

        match result {
            Ok(Ok(reply_bytes)) => Ok(reply_bytes),
            Ok(Err(_recv_err)) => Err(QueueError::ReplyDropped(correlation_id.to_string())),
            Err(_elapsed) => Err(QueueError::ReplyTimeout(correlation_id.to_string())),
        }
    }

    /// Deliver an inbound reply to a waiting `send_expect_reply` future.
    ///
    /// Called by the `InboundHandler` impl when an envelope arrives whose
    /// kind is a known reply (e.g. `DmInviteReply`) and whose
    /// `correlation_id` matches a pending request. Returns `true` if the
    /// oneshot was found and the reply forwarded; `false` if there's no
    /// pending request (likely the request timed out or this is a stray
    /// reply from a peer that resent).
    pub fn deliver_reply(&self, correlation_id: &str, reply_bytes: Vec<u8>) -> bool {
        let tx = self.inner.pending_replies.lock().remove(correlation_id);
        if let Some(tx) = tx {
            tx.send(reply_bytes).is_ok()
        } else {
            debug!(
                correlation_id,
                "deliver_reply: no pending request (already timed out or stray reply)"
            );
            false
        }
    }

    /// Access the underlying store. Used by [`crate::operations::calls`]
    /// to persist `PersistedCallState` and missed-call rows alongside
    /// envelope retry state.
    pub fn store(&self) -> &Arc<dyn EnvelopeStore> {
        &self.inner.store
    }

    /// Cancel all pending envelopes correlated to a given id. Called by
    /// call hangup paths so a torn-down call doesn't keep retrying.
    /// Returns the number of rows removed.
    pub async fn cancel_by_correlation(
        &self,
        correlation_id: &str,
    ) -> std::result::Result<usize, QueueError> {
        let n = self
            .inner
            .store
            .cancel_by_correlation(correlation_id)
            .await?;
        // Also drop any oneshot waiting on this correlation_id — the call
        // is gone, the reply (if any) is moot.
        self.inner.pending_replies.lock().remove(correlation_id);
        Ok(n)
    }

    /// Process all eligible envelopes. Called from the transport runtime
    /// loop on a 1 s cadence. Each tick processes up to
    /// [`TICK_BATCH_LIMIT`] rows; remaining rows pick up next tick.
    pub async fn run_retry_tick(&self) {
        let now_ms = rekindle_utils::timestamp_ms();
        let rows = match self
            .inner
            .store
            .load_eligible(&self.inner.owner_key, now_ms, TICK_BATCH_LIMIT)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "envelope_queue: load_eligible failed");
                return;
            }
        };

        for row in rows {
            self.attempt_send(row).await;
        }
    }

    async fn attempt_send(&self, row: PendingEnvelope) {
        let Some(type_id) = row.kind.wire_type_id() else {
            warn!(
                kind = ?row.kind,
                id = row.id,
                "envelope kind has no wire TypeId; dead-lettering",
            );
            let _ = self.inner.store.mark_dead(row.id).await;
            return;
        };

        // Look up the recipient's route blob and import.
        let target = match self.import_route_for(&row.recipient_key) {
            Ok(t) => t,
            Err(e) => {
                self.handle_failure(&row, format!("route lookup: {e}"))
                    .await;
                return;
            }
        };

        // Send via Sender::send_dm. The sender handles framing and signing.
        // W16.3 — pass through seq + correlation_id from the persisted row
        // so the signature covers them and the receiver can dedup.
        let send_result = self
            .inner
            .transport
            .sender()
            .send_dm(
                &target,
                type_id.class(),
                type_id,
                &self.inner.sender_secret,
                &self.inner.sender_public_hex,
                row.seq,
                row.correlation_id.as_deref(),
                &row.payload,
            )
            .await;

        match send_result {
            Ok(()) => self.handle_success(&row).await,
            Err(e) => self.handle_failure(&row, e.to_string()).await,
        }
    }

    /// W16.5b — synchronous RPC dispatch outside the persistence/retry
    /// queue. Used by [`crate::operations::calls::CallRuntime`] for the
    /// CallInvite handshake (Veilid `app_call`, 5-10 s budget; receiver
    /// replies synchronously inside `app_call_reply`).
    ///
    /// Unlike [`Self::send`], this method:
    /// - does NOT persist the request (no row in `pending_envelopes`),
    /// - does NOT retry on failure (caller handles `Err` via the state
    ///   machine's `LocalUnreachable` event),
    /// - returns the response payload bytes synchronously.
    ///
    /// Callers translate transport errors into domain-specific events
    /// (e.g. `CallEvent::LocalUnreachable { reason: "timeout" | ... }`).
    pub async fn send_app_call(
        &self,
        recipient: &str,
        type_id: TypeId,
        payload: &[u8],
        timeout: Duration,
    ) -> std::result::Result<Vec<u8>, TransportError> {
        let target = self.import_route_for(recipient)?;
        self.inner
            .transport
            .caller()
            .call_with_timeout(
                &target,
                type_id,
                &self.inner.sender_secret,
                &self.inner.sender_public_hex,
                payload,
                timeout,
            )
            .await
    }

    fn import_route_for(
        &self,
        recipient_key: &str,
    ) -> std::result::Result<crate::PeerTarget, TransportError> {
        let blob = {
            let peers = self.inner.transport.peers();
            let peers = peers.read();
            peers.get_route(recipient_key).map(<[u8]>::to_vec)
        };
        let blob = blob.ok_or_else(|| TransportError::NoRoute {
            peer: recipient_key.to_string(),
        })?;
        self.inner.transport.import_route(&blob)
    }

    async fn handle_success(&self, row: &PendingEnvelope) {
        if let Err(e) = self.inner.store.mark_delivered(row.id).await {
            warn!(error = %e, id = row.id, "mark_delivered failed");
        }
        self.inner
            .notifications
            .notify(&TransportNotification::EnvelopeDelivered {
                kind: row.kind.as_str().to_string(),
                correlation_id: row.correlation_id.clone(),
            });
        debug!(
            id = row.id,
            to = %row.recipient_key,
            kind = row.kind.as_str(),
            "envelope delivered",
        );
    }

    async fn handle_failure(&self, row: &PendingEnvelope, err: String) {
        let new_count = row.retry_count + 1;
        if new_count >= row.max_retries {
            // Cap hit. Dead-letter and notify.
            if let Err(e) = self.inner.store.mark_dead(row.id).await {
                warn!(error = %e, id = row.id, "mark_dead failed");
            }
            self.inner
                .notifications
                .notify(&TransportNotification::EnvelopeDeliveryFailed {
                    kind: row.kind.as_str().to_string(),
                    correlation_id: row.correlation_id.clone(),
                    last_error: err.clone(),
                });
            warn!(
                id = row.id,
                to = %row.recipient_key,
                kind = row.kind.as_str(),
                retries = new_count,
                error = %err,
                "envelope retry cap hit — dead-lettering",
            );
            return;
        }

        // Reschedule.
        let cfg = RetryConfig::for_kind(row.kind);
        let jitter_bp = rand::thread_rng().gen_range(0u32..=10_000);
        let backoff_ms = cfg.backoff_for_attempt(row.retry_count, jitter_bp);
        let next_retry_at = rekindle_utils::timestamp_ms() + backoff_ms;
        if let Err(e) = self
            .inner
            .store
            .mark_retry(row.id, new_count, next_retry_at, &err)
            .await
        {
            warn!(error = %e, id = row.id, "mark_retry failed");
        }
        debug!(
            id = row.id,
            to = %row.recipient_key,
            kind = row.kind.as_str(),
            retries = new_count,
            backoff_ms,
            error = %err,
            "envelope send failed, will retry",
        );
    }
}

// Re-export the kind/envelope types under the queue's module path so
// callers can write `envelope_queue::Kind` instead of
// `envelope_store::EnvelopeKind`. Optional aliasing — purely cosmetic.
pub use crate::envelope_store::{EnvelopeKind as Kind, PendingEnvelope as Envelope};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope_store::MemoryEnvelopeStore;

    #[tokio::test]
    async fn store_accepts_kind_with_no_wire_type_id() {
        // Decoupled sanity check: the store doesn't enforce
        // wire_type_id — that's the queue's job. Verify the store
        // happily round-trips a FriendRequestInbox row even though
        // the queue would refuse to enqueue it.
        let store = MemoryEnvelopeStore::new();
        assert!(EnvelopeKind::FriendRequestInbox.wire_type_id().is_none());
        let env = PendingEnvelope {
            id: 0,
            owner_key: "alice".into(),
            recipient_key: "bob".into(),
            kind: EnvelopeKind::FriendRequestInbox,
            seq: 1,
            correlation_id: None,
            payload: vec![1, 2, 3],
            created_at_ms: 100,
            next_retry_at_ms: 100,
            retry_count: 0,
            max_retries: 5,
            last_error: None,
        };
        store.enqueue(env).await.unwrap();
        let n = store.load_eligible("alice", 200, 64).await.unwrap().len();
        assert_eq!(n, 1);
    }

    #[test]
    fn retry_config_for_each_kind_matches_intent() {
        // Sanity: every variant is reachable from the const fn.
        let kinds = [
            EnvelopeKind::CallAccept,
            EnvelopeKind::CallAccept,
            EnvelopeKind::CallDecline,
            EnvelopeKind::CallEnd,
            EnvelopeKind::CallEnd,
            EnvelopeKind::CallMediaState,
            EnvelopeKind::CallReaction,
            EnvelopeKind::GroupCallOffer,
            EnvelopeKind::GroupCallAccept,
            EnvelopeKind::GroupCallDecline,
            EnvelopeKind::FriendRequestInbox,
            EnvelopeKind::FriendAcceptInbox,
            EnvelopeKind::DmMessage,
            EnvelopeKind::DmInviteRequest,
            EnvelopeKind::DmInviteReply,
            EnvelopeKind::GroupDmInviteRequest,
            EnvelopeKind::GroupDmInviteReply,
        ];
        for k in kinds {
            let cfg = RetryConfig::for_kind(k);
            assert!(cfg.max_retries > 0, "every kind has a retry budget: {k:?}");
            assert!(
                cfg.base_backoff_ms > 0,
                "every kind has a backoff seed: {k:?}"
            );
        }
    }
}

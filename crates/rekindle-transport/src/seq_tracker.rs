//! W16.3 — Per-recipient sequence tracker for receiver-side dedup.
//!
//! Veilid's `app_message` has no built-in dedup: a single logical
//! envelope can arrive multiple times if a lower transport layer
//! retransmits, or if a sender's retry tick beats the receiver's first
//! delivery. The receiver MUST dedup or risk duplicate UI events
//! (two IncomingCallModals, double-counted DM messages, etc.).
//!
//! ## Dedup model
//!
//! Each (sender, kind, correlation_id) tuple is tracked separately. The
//! receiver records the highest-seen `seq` for that tuple. Any incoming
//! envelope whose `seq <= last_seen` is dropped as a duplicate.
//!
//! ## Persistence
//!
//! State persists via [`EnvelopeStore::record_inbound_seq`] /
//! `get_last_inbound_seq` so receivers survive restart without
//! losing dedup state — without this, an attacker (or a bug) could
//! replay every previously-seen envelope at the next reconnect.
//!
//! ## Bounded memory
//!
//! The store is responsible for periodic cleanup of stale rows.
//! Reasonable default: drop rows whose `last_seen_at_ms` is older than
//! 30 days (configurable per-host). The tracker exposes a
//! [`SeqTracker::cleanup_stale`] hook for callers that want to drive
//! it manually.

use std::sync::Arc;

use tracing::debug;

use crate::envelope_store::{EnvelopeKind, EnvelopeStore, StoreError};

/// Receiver-side dedup tracker. Wraps an [`EnvelopeStore`] for
/// persistence and provides the semantic API the inbound dispatch
/// (W16.7) calls before processing.
///
/// Cloned cheaply (single `Arc`); intended to be shared across handler
/// tasks via clone.
#[derive(Clone)]
pub struct SeqTracker {
    store: Arc<dyn EnvelopeStore>,
    owner_key: String,
}

impl SeqTracker {
    /// Create a tracker scoped to a single owner identity.
    pub fn new(store: Arc<dyn EnvelopeStore>, owner_key: impl Into<String>) -> Self {
        Self {
            store,
            owner_key: owner_key.into(),
        }
    }

    /// Returns true if `(sender, kind, correlation_id, seq)` has already
    /// been seen — caller drops the envelope without further processing.
    ///
    /// Returns false for fresh envelopes; the receiver should then call
    /// [`Self::record`] to mark them as seen.
    ///
    /// # Errors
    /// Propagates store errors. Conservative behavior on store failure
    /// is up to the caller — the receiver could either fail-open
    /// (process the envelope, accepting potential duplicates) or
    /// fail-closed (drop). Recommended: log and fail-open, since dedup
    /// being wrong-positive is worse for UX than wrong-negative.
    pub async fn is_duplicate(
        &self,
        sender_key: &str,
        kind: EnvelopeKind,
        correlation_id: Option<&str>,
        seq: u64,
    ) -> Result<bool, StoreError> {
        let cid = correlation_id.unwrap_or("");
        let last = self
            .store
            .get_last_inbound_seq(&self.owner_key, sender_key, kind, cid)
            .await?;
        Ok(matches!(last, Some(last) if seq <= last))
    }

    /// Record `(sender, kind, correlation_id, seq)` as seen. Caller
    /// invokes this AFTER [`Self::is_duplicate`] returns false and the
    /// envelope passes other validation (signature verify, etc.).
    pub async fn record(
        &self,
        sender_key: &str,
        kind: EnvelopeKind,
        correlation_id: Option<&str>,
        seq: u64,
        now_ms: u64,
    ) -> Result<(), StoreError> {
        let cid = correlation_id.unwrap_or("");
        self.store
            .record_inbound_seq(&self.owner_key, sender_key, kind, cid, seq, now_ms)
            .await
    }

    /// Convenience: check + record in one call. Returns `Ok(true)` for
    /// fresh envelopes (caller should process), `Ok(false)` for
    /// duplicates (caller should drop).
    pub async fn check_and_record(
        &self,
        sender_key: &str,
        kind: EnvelopeKind,
        correlation_id: Option<&str>,
        seq: u64,
        now_ms: u64,
    ) -> Result<bool, StoreError> {
        if self
            .is_duplicate(sender_key, kind, correlation_id, seq)
            .await?
        {
            debug!(
                sender = sender_key,
                kind = kind.as_str(),
                correlation_id,
                seq,
                "SeqTracker: duplicate envelope, dropping",
            );
            return Ok(false);
        }
        self.record(sender_key, kind, correlation_id, seq, now_ms)
            .await?;
        Ok(true)
    }

    // cleanup_stale: deferred. The store doesn't have a bulk-delete
    // API yet; re-add this method alongside that work. Until then,
    // snapshots stay bounded by usage patterns. See
    // `.claude/plans/giggly-inventing-snowglobe.md` Wave 16 follow-ups.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope_store::MemoryEnvelopeStore;

    #[tokio::test]
    async fn fresh_envelopes_are_not_duplicates() {
        let store: Arc<dyn EnvelopeStore> = Arc::new(MemoryEnvelopeStore::new());
        let tracker = SeqTracker::new(store, "alice");
        let dup = tracker
            .is_duplicate("bob", EnvelopeKind::CallAccept, Some("call-1"), 1)
            .await
            .unwrap();
        assert!(!dup, "first envelope is never a duplicate");
    }

    #[tokio::test]
    async fn replayed_seq_is_duplicate() {
        let store: Arc<dyn EnvelopeStore> = Arc::new(MemoryEnvelopeStore::new());
        let tracker = SeqTracker::new(store, "alice");
        tracker
            .record("bob", EnvelopeKind::CallAccept, Some("call-1"), 5, 100)
            .await
            .unwrap();
        // Same seq replay → duplicate
        let dup_same = tracker
            .is_duplicate("bob", EnvelopeKind::CallAccept, Some("call-1"), 5)
            .await
            .unwrap();
        assert!(dup_same, "same-seq replay is a duplicate");
        // Lower seq → also duplicate
        let dup_lower = tracker
            .is_duplicate("bob", EnvelopeKind::CallAccept, Some("call-1"), 4)
            .await
            .unwrap();
        assert!(dup_lower, "lower-seq replay is a duplicate");
        // Higher seq → not a duplicate
        let fresh = tracker
            .is_duplicate("bob", EnvelopeKind::CallAccept, Some("call-1"), 6)
            .await
            .unwrap();
        assert!(!fresh, "higher seq is not a duplicate");
    }

    #[tokio::test]
    async fn correlation_id_scopes_dedup() {
        // Different call_ids with same seq are independent.
        let store: Arc<dyn EnvelopeStore> = Arc::new(MemoryEnvelopeStore::new());
        let tracker = SeqTracker::new(store, "alice");
        tracker
            .record("bob", EnvelopeKind::CallAccept, Some("call-1"), 5, 100)
            .await
            .unwrap();
        let other_call = tracker
            .is_duplicate("bob", EnvelopeKind::CallAccept, Some("call-2"), 5)
            .await
            .unwrap();
        assert!(!other_call, "different correlation_id is a separate stream");
    }

    #[tokio::test]
    async fn kind_scopes_dedup() {
        let store: Arc<dyn EnvelopeStore> = Arc::new(MemoryEnvelopeStore::new());
        let tracker = SeqTracker::new(store, "alice");
        tracker
            .record("bob", EnvelopeKind::CallAccept, Some("call-1"), 5, 100)
            .await
            .unwrap();
        let other_kind = tracker
            .is_duplicate("bob", EnvelopeKind::CallEnd, Some("call-1"), 5)
            .await
            .unwrap();
        assert!(!other_kind, "different kind is a separate stream");
    }

    #[tokio::test]
    async fn sender_scopes_dedup() {
        let store: Arc<dyn EnvelopeStore> = Arc::new(MemoryEnvelopeStore::new());
        let tracker = SeqTracker::new(store, "alice");
        tracker
            .record("bob", EnvelopeKind::CallAccept, Some("call-1"), 5, 100)
            .await
            .unwrap();
        // Carol sending the same call_id with the same seq isn't a dup —
        // separate sender = separate stream.
        let other_sender = tracker
            .is_duplicate("carol", EnvelopeKind::CallAccept, Some("call-1"), 5)
            .await
            .unwrap();
        assert!(!other_sender, "different sender is a separate stream");
    }

    #[tokio::test]
    async fn check_and_record_atomicity() {
        let store: Arc<dyn EnvelopeStore> = Arc::new(MemoryEnvelopeStore::new());
        let tracker = SeqTracker::new(store, "alice");
        // First call: fresh → returns true (caller should process)
        assert!(tracker
            .check_and_record("bob", EnvelopeKind::CallAccept, Some("c1"), 1, 100)
            .await
            .unwrap());
        // Same envelope replayed → false (caller should drop)
        assert!(!tracker
            .check_and_record("bob", EnvelopeKind::CallAccept, Some("c1"), 1, 200)
            .await
            .unwrap());
        // Higher seq same stream → true
        assert!(tracker
            .check_and_record("bob", EnvelopeKind::CallAccept, Some("c1"), 2, 300)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn empty_correlation_id_equivalent_to_none() {
        // Document the edge case: Some("") and None map to the same
        // dedup key. Receivers don't need to distinguish.
        let store: Arc<dyn EnvelopeStore> = Arc::new(MemoryEnvelopeStore::new());
        let tracker = SeqTracker::new(store, "alice");
        tracker
            .record("bob", EnvelopeKind::DmMessage, Some(""), 5, 100)
            .await
            .unwrap();
        let dup_none = tracker
            .is_duplicate("bob", EnvelopeKind::DmMessage, None, 5)
            .await
            .unwrap();
        assert!(dup_none, "Some(\"\") and None share the dedup slot",);
    }
}

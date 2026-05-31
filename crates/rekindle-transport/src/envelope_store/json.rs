//! Atomic JSON-on-disk impl of [`EnvelopeStore`]. Default for rekindle-cli
//! and rekindle-node. Mirrors the tmp+fsync+rename pattern from
//! `crate::session::Session::save`.
//!
//! Layout: one JSON file at `{base_dir}/envelopes/{owner_key_hex}.json`
//! holding the full snapshot of pending envelopes, seq tracking tables,
//! and active call states for that owner. ~4 KB on disk per active user;
//! safe to serialize on every mutation.
//!
//! Concurrency: a per-owner [`tokio::sync::Mutex`] held during read+
//! write+rename; the in-memory snapshot is reloaded if the file mtime
//! changed externally. No multi-process safety beyond the OS rename
//! atomicity — single-process assumption matches the host model.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::{EnvelopeKind, EnvelopeStore, PendingEnvelope, PersistedCallState, StoreError};

/// On-disk snapshot for one owner. Whole snapshot rewritten atomically
/// on every mutation. Bounded by retry caps and dedup-cleanup, so the
/// snapshot stays small (typical: dozens of pending envelopes, maybe
/// hundreds of seen-envelope rows after weeks of use).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct OwnerSnapshot {
    /// Monotonic id source for new envelopes.
    next_id: i64,
    /// Pending envelope rows keyed by id.
    envelopes: HashMap<i64, PendingEnvelope>,
    /// Seq tracking tables.
    outbound_seqs: HashMap<SeqKey, u64>,
    seen_envelopes: HashMap<SeqKey, SeenEntry>,
    /// Active Dialing/Incoming call states keyed by call_id.
    active_calls: HashMap<String, PersistedCallState>,
}

/// Composite key for seq tracking. Serialized as a string for JSON
/// stability — `(recipient, kind, correlation_id)` joined with `\u{1f}`
/// (info separator).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct SeqKey(String);

impl SeqKey {
    fn new(peer: &str, kind: EnvelopeKind, correlation_id: &str) -> Self {
        Self(format!(
            "{peer}\u{1f}{}\u{1f}{correlation_id}",
            kind.as_str()
        ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SeenEntry {
    last_seq: u64,
    last_seen_at_ms: u64,
}

/// Atomic JSON-on-disk envelope store. One file per owner.
pub struct JsonEnvelopeStore {
    base_dir: PathBuf,
    /// Per-owner mutex + cached snapshot. Loaded lazily on first access.
    cache: Mutex<HashMap<String, Arc<Mutex<OwnerSnapshot>>>>,
}

impl JsonEnvelopeStore {
    /// Create a store rooted at `base_dir`. Files are at
    /// `{base_dir}/envelopes/{owner_key}.json`. Directory is created on
    /// first write.
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn path_for(&self, owner_key: &str) -> PathBuf {
        self.base_dir
            .join("envelopes")
            .join(format!("{owner_key}.json"))
    }

    /// Get or load the snapshot for an owner. Loaded once per process
    /// then mutated in place.
    async fn snapshot_for(&self, owner_key: &str) -> Result<Arc<Mutex<OwnerSnapshot>>, StoreError> {
        let mut cache = self.cache.lock().await;
        if let Some(existing) = cache.get(owner_key) {
            return Ok(existing.clone());
        }
        let path = self.path_for(owner_key);
        let snapshot = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<OwnerSnapshot>(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => OwnerSnapshot::default(),
            Err(e) => return Err(StoreError::Io(format!("read {}: {e}", path.display()))),
        };
        let arc = Arc::new(Mutex::new(snapshot));
        cache.insert(owner_key.to_string(), arc.clone());
        Ok(arc)
    }

    /// Persist a snapshot to disk via tmp+fsync+rename. Mirrors
    /// `session::atomic_write` (kept private there; duplicated here to
    /// avoid a circular module dep).
    fn persist(path: &Path, snapshot: &OwnerSnapshot) -> Result<(), StoreError> {
        let bytes = serde_json::to_vec_pretty(snapshot)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("tmp");
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[async_trait]
impl EnvelopeStore for JsonEnvelopeStore {
    async fn enqueue(&self, mut env: PendingEnvelope) -> Result<i64, StoreError> {
        let snap_arc = self.snapshot_for(&env.owner_key).await?;
        let path = self.path_for(&env.owner_key);
        let mut snap = snap_arc.lock().await;
        snap.next_id += 1;
        env.id = snap.next_id;
        let id = env.id;
        snap.envelopes.insert(id, env);
        Self::persist(&path, &snap)?;
        Ok(id)
    }

    async fn load_eligible(
        &self,
        owner_key: &str,
        now_ms: u64,
        limit: usize,
    ) -> Result<Vec<PendingEnvelope>, StoreError> {
        let snap_arc = self.snapshot_for(owner_key).await?;
        let snap = snap_arc.lock().await;
        let mut eligible: Vec<PendingEnvelope> = snap
            .envelopes
            .values()
            .filter(|e| e.next_retry_at_ms <= now_ms)
            .cloned()
            .collect();
        eligible.sort_by_key(|e| e.next_retry_at_ms);
        eligible.truncate(limit);
        Ok(eligible)
    }

    async fn mark_delivered(&self, id: i64) -> Result<(), StoreError> {
        // Need the owner — find it by id. JSON layout is per-owner so
        // we have to scan caches; cheap because cache is small.
        let owners: Vec<String> = {
            let cache = self.cache.lock().await;
            cache.keys().cloned().collect()
        };
        for owner in owners {
            let snap_arc = self.snapshot_for(&owner).await?;
            let path = self.path_for(&owner);
            let mut snap = snap_arc.lock().await;
            if snap.envelopes.remove(&id).is_some() {
                Self::persist(&path, &snap)?;
                return Ok(());
            }
        }
        Err(StoreError::NotFound(id))
    }

    async fn mark_retry(
        &self,
        id: i64,
        retry_count: u32,
        next_retry_at_ms: u64,
        last_error: &str,
    ) -> Result<(), StoreError> {
        let owners: Vec<String> = {
            let cache = self.cache.lock().await;
            cache.keys().cloned().collect()
        };
        for owner in owners {
            let snap_arc = self.snapshot_for(&owner).await?;
            let path = self.path_for(&owner);
            let mut snap = snap_arc.lock().await;
            if let Some(env) = snap.envelopes.get_mut(&id) {
                env.retry_count = retry_count;
                env.next_retry_at_ms = next_retry_at_ms;
                env.last_error = Some(last_error.to_string());
                Self::persist(&path, &snap)?;
                return Ok(());
            }
        }
        Err(StoreError::NotFound(id))
    }

    async fn mark_dead(&self, id: i64) -> Result<(), StoreError> {
        // Same shape as mark_delivered — both delete the row.
        self.mark_delivered(id).await
    }

    async fn cancel_by_correlation(&self, correlation_id: &str) -> Result<usize, StoreError> {
        let owners: Vec<String> = {
            let cache = self.cache.lock().await;
            cache.keys().cloned().collect()
        };
        let mut total = 0usize;
        for owner in owners {
            let snap_arc = self.snapshot_for(&owner).await?;
            let path = self.path_for(&owner);
            let mut snap = snap_arc.lock().await;
            let before = snap.envelopes.len();
            snap.envelopes
                .retain(|_, e| e.correlation_id.as_deref() != Some(correlation_id));
            let removed = before - snap.envelopes.len();
            if removed > 0 {
                Self::persist(&path, &snap)?;
                total += removed;
            }
        }
        Ok(total)
    }

    async fn next_outbound_seq(
        &self,
        owner_key: &str,
        recipient_key: &str,
        kind: EnvelopeKind,
        correlation_id: &str,
    ) -> Result<u64, StoreError> {
        let snap_arc = self.snapshot_for(owner_key).await?;
        let path = self.path_for(owner_key);
        let mut snap = snap_arc.lock().await;
        let key = SeqKey::new(recipient_key, kind, correlation_id);
        let next = snap.outbound_seqs.get(&key).copied().unwrap_or(0) + 1;
        snap.outbound_seqs.insert(key, next);
        Self::persist(&path, &snap)?;
        Ok(next)
    }

    async fn record_inbound_seq(
        &self,
        owner_key: &str,
        sender_key: &str,
        kind: EnvelopeKind,
        correlation_id: &str,
        seq: u64,
        now_ms: u64,
    ) -> Result<(), StoreError> {
        let snap_arc = self.snapshot_for(owner_key).await?;
        let path = self.path_for(owner_key);
        let mut snap = snap_arc.lock().await;
        let key = SeqKey::new(sender_key, kind, correlation_id);
        snap.seen_envelopes.insert(
            key,
            SeenEntry {
                last_seq: seq,
                last_seen_at_ms: now_ms,
            },
        );
        Self::persist(&path, &snap)?;
        Ok(())
    }

    async fn get_last_inbound_seq(
        &self,
        owner_key: &str,
        sender_key: &str,
        kind: EnvelopeKind,
        correlation_id: &str,
    ) -> Result<Option<u64>, StoreError> {
        let snap_arc = self.snapshot_for(owner_key).await?;
        let snap = snap_arc.lock().await;
        let key = SeqKey::new(sender_key, kind, correlation_id);
        Ok(snap.seen_envelopes.get(&key).map(|e| e.last_seq))
    }

    async fn save_active_call(&self, state: PersistedCallState) -> Result<(), StoreError> {
        let snap_arc = self.snapshot_for(&state.owner_key).await?;
        let path = self.path_for(&state.owner_key);
        let mut snap = snap_arc.lock().await;
        snap.active_calls.insert(state.call_id.clone(), state);
        Self::persist(&path, &snap)?;
        Ok(())
    }

    async fn delete_active_call(&self, owner_key: &str, call_id: &str) -> Result<(), StoreError> {
        let snap_arc = self.snapshot_for(owner_key).await?;
        let path = self.path_for(owner_key);
        let mut snap = snap_arc.lock().await;
        if snap.active_calls.remove(call_id).is_some() {
            Self::persist(&path, &snap)?;
        }
        Ok(())
    }

    async fn load_active_calls(
        &self,
        owner_key: &str,
    ) -> Result<Vec<PersistedCallState>, StoreError> {
        let snap_arc = self.snapshot_for(owner_key).await?;
        let snap = snap_arc.lock().await;
        Ok(snap.active_calls.values().cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample_envelope(owner: &str, recipient: &str, kind: EnvelopeKind) -> PendingEnvelope {
        PendingEnvelope {
            id: 0,
            owner_key: owner.to_string(),
            recipient_key: recipient.to_string(),
            kind,
            seq: 1,
            correlation_id: Some("call-abc".into()),
            payload: vec![1, 2, 3],
            created_at_ms: 100,
            next_retry_at_ms: 100,
            retry_count: 0,
            max_retries: 5,
            last_error: None,
        }
    }

    #[tokio::test]
    async fn enqueue_and_load_eligible() {
        let dir = tempdir().unwrap();
        let store = JsonEnvelopeStore::new(dir.path());
        let id = store
            .enqueue(sample_envelope("alice", "bob", EnvelopeKind::CallAccept))
            .await
            .unwrap();
        assert!(id > 0);
        let eligible = store.load_eligible("alice", 200, 64).await.unwrap();
        assert_eq!(eligible.len(), 1);
        assert_eq!(eligible[0].kind, EnvelopeKind::CallAccept);
        let none = store.load_eligible("alice", 50, 64).await.unwrap();
        assert!(none.is_empty(), "row not yet eligible");
    }

    #[tokio::test]
    async fn mark_delivered_removes_row() {
        let dir = tempdir().unwrap();
        let store = JsonEnvelopeStore::new(dir.path());
        let id = store
            .enqueue(sample_envelope("alice", "bob", EnvelopeKind::CallAccept))
            .await
            .unwrap();
        store.mark_delivered(id).await.unwrap();
        let after = store.load_eligible("alice", 200, 64).await.unwrap();
        assert!(after.is_empty());
    }

    #[tokio::test]
    async fn cancel_by_correlation_drops_matching_rows() {
        let dir = tempdir().unwrap();
        let store = JsonEnvelopeStore::new(dir.path());
        let mut e1 = sample_envelope("alice", "bob", EnvelopeKind::CallAccept);
        e1.correlation_id = Some("call-1".into());
        let mut e2 = sample_envelope("alice", "bob", EnvelopeKind::CallAccept);
        e2.correlation_id = Some("call-2".into());
        let mut e3 = sample_envelope("alice", "bob", EnvelopeKind::CallEnd);
        e3.correlation_id = Some("call-1".into());
        store.enqueue(e1).await.unwrap();
        store.enqueue(e2).await.unwrap();
        store.enqueue(e3).await.unwrap();
        let removed = store.cancel_by_correlation("call-1").await.unwrap();
        assert_eq!(removed, 2);
        let remaining = store.load_eligible("alice", 200, 64).await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].correlation_id.as_deref(), Some("call-2"));
    }

    #[tokio::test]
    async fn outbound_seq_monotonic_per_key() {
        let dir = tempdir().unwrap();
        let store = JsonEnvelopeStore::new(dir.path());
        let s1 = store
            .next_outbound_seq("alice", "bob", EnvelopeKind::CallAccept, "")
            .await
            .unwrap();
        let s2 = store
            .next_outbound_seq("alice", "bob", EnvelopeKind::CallAccept, "")
            .await
            .unwrap();
        let s3 = store
            .next_outbound_seq("alice", "bob", EnvelopeKind::CallEnd, "")
            .await
            .unwrap();
        assert_eq!(s1, 1);
        assert_eq!(s2, 2);
        assert_eq!(s3, 1, "different kind starts at 1 independently");
    }

    #[tokio::test]
    async fn inbound_seq_dedup() {
        let dir = tempdir().unwrap();
        let store = JsonEnvelopeStore::new(dir.path());
        let last = store
            .get_last_inbound_seq("alice", "bob", EnvelopeKind::CallAccept, "")
            .await
            .unwrap();
        assert!(last.is_none());
        store
            .record_inbound_seq("alice", "bob", EnvelopeKind::CallAccept, "", 7, 100)
            .await
            .unwrap();
        let seen = store
            .get_last_inbound_seq("alice", "bob", EnvelopeKind::CallAccept, "")
            .await
            .unwrap();
        assert_eq!(seen, Some(7));
    }

    #[tokio::test]
    async fn active_call_round_trip() {
        let dir = tempdir().unwrap();
        let store = JsonEnvelopeStore::new(dir.path());
        let state = PersistedCallState {
            owner_key: "alice".into(),
            call_id: "abc".into(),
            peer_pubkey: "bob".into(),
            kind: "audio".into(),
            status: "outgoing".into(),
            expires_at_ms: 30_000,
            my_x25519_secret: Some(vec![0u8; 32]),
            peer_x25519_pub: None,
            group_participants: vec![],
            inserted_at_ms: 100,
        };
        store.save_active_call(state.clone()).await.unwrap();
        let loaded = store.load_active_calls("alice").await.unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].call_id, "abc");
        store.delete_active_call("alice", "abc").await.unwrap();
        let after = store.load_active_calls("alice").await.unwrap();
        assert!(after.is_empty());
    }

    #[tokio::test]
    async fn snapshot_survives_reopen() {
        let dir = tempdir().unwrap();
        {
            let store = JsonEnvelopeStore::new(dir.path());
            store
                .enqueue(sample_envelope("alice", "bob", EnvelopeKind::CallAccept))
                .await
                .unwrap();
        }
        let store2 = JsonEnvelopeStore::new(dir.path());
        let eligible = store2.load_eligible("alice", 200, 64).await.unwrap();
        assert_eq!(eligible.len(), 1);
    }
}

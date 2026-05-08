//! In-memory [`EnvelopeStore`] for tests. No I/O. No persistence — every
//! row is dropped when the struct is dropped. Used in unit tests across
//! the workspace and as a stand-in when the host hasn't wired a real
//! store yet.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use super::{EnvelopeKind, EnvelopeStore, PendingEnvelope, PersistedCallState, StoreError};

/// Composite key used by the seq-tracking tables. Same shape as the JSON
/// impl's `SeqKey` but kept private to each impl since they don't share
/// the Hash bound shape.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SeqKey {
    peer: String,
    kind: EnvelopeKind,
    correlation_id: String,
}

impl SeqKey {
    fn new(peer: &str, kind: EnvelopeKind, correlation_id: &str) -> Self {
        Self {
            peer: peer.to_string(),
            kind,
            correlation_id: correlation_id.to_string(),
        }
    }
}

#[derive(Debug, Default)]
struct Inner {
    next_id: i64,
    /// Per-owner pending envelopes.
    envelopes: HashMap<String, HashMap<i64, PendingEnvelope>>,
    /// Per-owner seq tracking.
    outbound: HashMap<String, HashMap<SeqKey, u64>>,
    inbound: HashMap<String, HashMap<SeqKey, (u64, u64)>>, // (last_seq, last_seen_at_ms)
    /// Per-owner active calls.
    calls: HashMap<String, HashMap<String, PersistedCallState>>,
}

/// In-memory envelope store. Cheap to construct; cloneable via `Arc`.
#[derive(Debug, Default, Clone)]
pub struct MemoryEnvelopeStore {
    inner: Arc<Mutex<Inner>>,
}

impl MemoryEnvelopeStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl EnvelopeStore for MemoryEnvelopeStore {
    async fn enqueue(&self, mut env: PendingEnvelope) -> Result<i64, StoreError> {
        let mut inner = self.inner.lock().await;
        inner.next_id += 1;
        env.id = inner.next_id;
        let id = env.id;
        inner
            .envelopes
            .entry(env.owner_key.clone())
            .or_default()
            .insert(id, env);
        Ok(id)
    }

    async fn load_eligible(
        &self,
        owner_key: &str,
        now_ms: u64,
        limit: usize,
    ) -> Result<Vec<PendingEnvelope>, StoreError> {
        let inner = self.inner.lock().await;
        let Some(rows) = inner.envelopes.get(owner_key) else {
            return Ok(vec![]);
        };
        let mut eligible: Vec<PendingEnvelope> = rows
            .values()
            .filter(|e| e.next_retry_at_ms <= now_ms)
            .cloned()
            .collect();
        eligible.sort_by_key(|e| e.next_retry_at_ms);
        eligible.truncate(limit);
        Ok(eligible)
    }

    async fn mark_delivered(&self, id: i64) -> Result<(), StoreError> {
        let mut inner = self.inner.lock().await;
        for rows in inner.envelopes.values_mut() {
            if rows.remove(&id).is_some() {
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
        let mut inner = self.inner.lock().await;
        for rows in inner.envelopes.values_mut() {
            if let Some(env) = rows.get_mut(&id) {
                env.retry_count = retry_count;
                env.next_retry_at_ms = next_retry_at_ms;
                env.last_error = Some(last_error.to_string());
                return Ok(());
            }
        }
        Err(StoreError::NotFound(id))
    }

    async fn mark_dead(&self, id: i64) -> Result<(), StoreError> {
        self.mark_delivered(id).await
    }

    async fn cancel_by_correlation(
        &self,
        correlation_id: &str,
    ) -> Result<usize, StoreError> {
        let mut inner = self.inner.lock().await;
        let mut removed = 0usize;
        for rows in inner.envelopes.values_mut() {
            let before = rows.len();
            rows.retain(|_, e| e.correlation_id.as_deref() != Some(correlation_id));
            removed += before - rows.len();
        }
        Ok(removed)
    }

    async fn next_outbound_seq(
        &self,
        owner_key: &str,
        recipient_key: &str,
        kind: EnvelopeKind,
        correlation_id: &str,
    ) -> Result<u64, StoreError> {
        let mut inner = self.inner.lock().await;
        let table = inner.outbound.entry(owner_key.to_string()).or_default();
        let key = SeqKey::new(recipient_key, kind, correlation_id);
        let next = table.get(&key).copied().unwrap_or(0) + 1;
        table.insert(key, next);
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
        let mut inner = self.inner.lock().await;
        let table = inner.inbound.entry(owner_key.to_string()).or_default();
        table.insert(SeqKey::new(sender_key, kind, correlation_id), (seq, now_ms));
        Ok(())
    }

    async fn get_last_inbound_seq(
        &self,
        owner_key: &str,
        sender_key: &str,
        kind: EnvelopeKind,
        correlation_id: &str,
    ) -> Result<Option<u64>, StoreError> {
        let inner = self.inner.lock().await;
        Ok(inner
            .inbound
            .get(owner_key)
            .and_then(|t| t.get(&SeqKey::new(sender_key, kind, correlation_id)))
            .map(|(seq, _)| *seq))
    }

    async fn save_active_call(&self, state: PersistedCallState) -> Result<(), StoreError> {
        let mut inner = self.inner.lock().await;
        inner
            .calls
            .entry(state.owner_key.clone())
            .or_default()
            .insert(state.call_id.clone(), state);
        Ok(())
    }

    async fn delete_active_call(
        &self,
        owner_key: &str,
        call_id: &str,
    ) -> Result<(), StoreError> {
        let mut inner = self.inner.lock().await;
        if let Some(rows) = inner.calls.get_mut(owner_key) {
            rows.remove(call_id);
        }
        Ok(())
    }

    async fn load_active_calls(
        &self,
        owner_key: &str,
    ) -> Result<Vec<PersistedCallState>, StoreError> {
        let inner = self.inner.lock().await;
        Ok(inner
            .calls
            .get(owner_key)
            .map(|t| t.values().cloned().collect())
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(owner: &str) -> PendingEnvelope {
        PendingEnvelope {
            id: 0,
            owner_key: owner.to_string(),
            recipient_key: "bob".into(),
            kind: EnvelopeKind::CallInvite,
            seq: 1,
            correlation_id: Some("call-x".into()),
            payload: vec![1, 2, 3],
            created_at_ms: 100,
            next_retry_at_ms: 100,
            retry_count: 0,
            max_retries: 5,
            last_error: None,
        }
    }

    #[tokio::test]
    async fn enqueue_assigns_increasing_ids() {
        let store = MemoryEnvelopeStore::new();
        let a = store.enqueue(sample("alice")).await.unwrap();
        let b = store.enqueue(sample("alice")).await.unwrap();
        assert!(b > a);
    }

    #[tokio::test]
    async fn mark_delivered_then_not_found() {
        let store = MemoryEnvelopeStore::new();
        let id = store.enqueue(sample("alice")).await.unwrap();
        store.mark_delivered(id).await.unwrap();
        let res = store.mark_delivered(id).await;
        assert!(matches!(res, Err(StoreError::NotFound(_))));
    }

    #[tokio::test]
    async fn dedup_round_trip() {
        let store = MemoryEnvelopeStore::new();
        assert!(store
            .get_last_inbound_seq("a", "b", EnvelopeKind::CallInvite, "")
            .await
            .unwrap()
            .is_none());
        store
            .record_inbound_seq("a", "b", EnvelopeKind::CallInvite, "", 5, 100)
            .await
            .unwrap();
        assert_eq!(
            store
                .get_last_inbound_seq("a", "b", EnvelopeKind::CallInvite, "")
                .await
                .unwrap(),
            Some(5)
        );
    }
}

//! Phase 14.r split — `CallRegistry` + `GroupCallRegistry` trait impls
//! over the `Arc<Mutex<HashMap<call_id, _>>>` types held on AppState.
//!
//! Both registries are deliberately thin: lock the underlying map, do
//! one HashMap op, return a clone (because the crate-side trait
//! contract is "value-out" — keeps the caller from holding a guard
//! across `.await`).

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use rekindle_calls::group_state::{GroupCallState, GroupCallStatus};
use rekindle_calls::signaling::registry::GroupCallSnapshot;
use rekindle_calls::signaling::{CallRegistry, GroupCallRegistry};
use rekindle_calls::state::CallState;

pub struct ActiveCallRegistry {
    inner: Arc<Mutex<HashMap<String, CallState>>>,
}

impl ActiveCallRegistry {
    pub fn new(inner: Arc<Mutex<HashMap<String, CallState>>>) -> Self {
        Self { inner }
    }
}

impl CallRegistry for ActiveCallRegistry {
    fn insert(&self, call: CallState) {
        self.inner.lock().insert(call.call_id.clone(), call);
    }

    fn get(&self, call_id: &str) -> Option<CallState> {
        self.inner.lock().get(call_id).cloned()
    }

    fn remove(&self, call_id: &str) -> Option<CallState> {
        self.inner.lock().remove(call_id)
    }

    fn contains(&self, call_id: &str) -> bool {
        self.inner.lock().contains_key(call_id)
    }

    fn outgoing_to_peer(&self, peer_pubkey_hex: &str) -> Option<CallState> {
        use rekindle_calls::state::CallStatus;
        self.inner
            .lock()
            .values()
            .find(|c| {
                c.peer_pubkey == peer_pubkey_hex && matches!(c.status, CallStatus::Outgoing)
            })
            .cloned()
    }

    fn list_all(&self) -> Vec<CallState> {
        self.inner.lock().values().cloned().collect()
    }
}

pub struct ActiveGroupCallRegistry {
    inner: Arc<Mutex<HashMap<String, GroupCallState>>>,
}

impl ActiveGroupCallRegistry {
    pub fn new(inner: Arc<Mutex<HashMap<String, GroupCallState>>>) -> Self {
        Self { inner }
    }
}

impl GroupCallRegistry for ActiveGroupCallRegistry {
    fn insert(&self, call: GroupCallState) {
        self.inner.lock().insert(call.call_id.clone(), call);
    }

    fn remove(&self, call_id: &str) -> Option<GroupCallState> {
        self.inner.lock().remove(call_id)
    }

    fn contains(&self, call_id: &str) -> bool {
        self.inner.lock().contains_key(call_id)
    }

    fn add_accept(&self, call_id: &str, peer_pubkey_hex: &str) -> bool {
        let mut guard = self.inner.lock();
        let Some(call) = guard.get_mut(call_id) else {
            return false;
        };
        let was_empty = call.accepted.is_empty();
        call.accepted.insert(peer_pubkey_hex.to_string());
        was_empty
    }

    fn set_status(&self, call_id: &str, status: GroupCallStatus) {
        if let Some(call) = self.inner.lock().get_mut(call_id) {
            call.status = status;
        }
    }

    fn snapshot(&self, call_id: &str) -> Option<GroupCallSnapshot> {
        let guard = self.inner.lock();
        let call = guard.get(call_id)?;
        Some(GroupCallSnapshot {
            call_id: call.call_id.clone(),
            initiator_pubkey: call.initiator_pubkey.clone(),
            kind: call.kind,
            participants: call.participants.clone(),
            accepted_count: call.accepted.len(),
            status: call.status,
        })
    }
}

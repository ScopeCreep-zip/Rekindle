//! Phase 14 — call registry ports.
//!
//! `CallRegistry` (1:1) and `GroupCallRegistry` (1:N) are the
//! storage ports the rekindle-calls signaling logic uses. The adapter
//! holds the actual `Mutex<HashMap<...>>` (currently on `AppState`
//! pre-Phase-14; will move into the adapter's owned state in 14.h).
//!
//! Methods are all sync + take/return by value (cloning) so the trait
//! stays object-safe (`Arc<dyn CallRegistry>`). `CallState` and
//! `GroupCallState` derive `Clone`; the X25519 secret + call_key are
//! held in types that zeroize on drop, so cloning then dropping
//! preserves the wipe semantics — the clone's eventual Drop runs the
//! zeroize.

use crate::group_state::{GroupCallState, GroupCallStatus};
use crate::state::CallState;

/// 1:1 call registry. Backed by a `Mutex<HashMap<call_id, CallState>>`
/// in the src-tauri adapter (was `AppState.active_calls` pre-Phase-14).
pub trait CallRegistry: Send + Sync {
    fn insert(&self, call: CallState);
    fn get(&self, call_id: &str) -> Option<CallState>;
    fn remove(&self, call_id: &str) -> Option<CallState>;
    fn contains(&self, call_id: &str) -> bool;
    /// Find any outgoing call to the given peer (used for glare
    /// resolution — W13.15). Returns the first match.
    fn outgoing_to_peer(&self, peer_pubkey_hex: &str) -> Option<CallState>;
    /// Snapshot of all calls (for debug + diagnostics commands).
    fn list_all(&self) -> Vec<CallState>;
}

/// Group call registry. Backed by `Mutex<HashMap<call_id, GroupCallState>>`
/// (was `AppState.group_calls`).
///
/// `GroupCallState` is NOT `Clone` (it contains `Option<StaticSecret>`
/// which has `Drop` semantics for zeroization but no `Clone` impl in
/// the path we use). The trait surface uses callback patterns
/// (`with_mut`) where state needs to be modified in-place, and
/// dedicated mutation methods (`set_status`, `add_accept`) for the
/// common cases. `get_clone_lite` returns a partial snapshot (no
/// secret) for diagnostic reads.
pub trait GroupCallRegistry: Send + Sync {
    fn insert(&self, call: GroupCallState);
    fn remove(&self, call_id: &str) -> Option<GroupCallState>;
    fn contains(&self, call_id: &str) -> bool;
    /// Record a participant's accept. Returns `true` if this was the
    /// FIRST accept (the initiator transitions Outgoing → Active on
    /// the first accept).
    fn add_accept(&self, call_id: &str, peer_pubkey_hex: &str) -> bool;
    /// Set the call's status.
    fn set_status(&self, call_id: &str, status: GroupCallStatus);
    /// Read-only diagnostic snapshot: returns the participant list +
    /// status + accepted count, omitting the X25519 secret and call_key.
    fn snapshot(&self, call_id: &str) -> Option<GroupCallSnapshot>;
}

/// Diagnostic-safe snapshot of a `GroupCallState` (no key material).
#[derive(Debug, Clone)]
pub struct GroupCallSnapshot {
    pub call_id: String,
    pub initiator_pubkey: String,
    pub kind: u8,
    pub participants: Vec<String>,
    pub accepted_count: usize,
    pub status: GroupCallStatus,
}

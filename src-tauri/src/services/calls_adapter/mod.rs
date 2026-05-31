//! Phase 14 — call signaling adapter.
//!
//! Implements `rekindle_calls::signaling::CallSignalingDeps` against
//! the live `AppState` + `tauri::AppHandle` + `DbPool`. The crate's
//! 1:1 and group signaling handlers (handle_incoming_invite,
//! handle_group_call_payload, ring timers, etc.) parameterise over
//! this trait so the protocol logic stays free of Tauri/Veilid
//! concerns.
//!
//! Phase 14.r split layout (≤500 LoC per file):
//! * [`registry`] — `ActiveCallRegistry` + `ActiveGroupCallRegistry` (the
//!   `CallRegistry` / `GroupCallRegistry` trait impls over
//!   `Arc<Mutex<HashMap>>`).
//! * [`deps_impl`] — the full `impl CallSignalingDeps for CallsAdapter`
//!   block (~456 LoC).

use std::sync::Arc;

use rekindle_calls::signaling::{CallRegistry, GroupCallRegistry};

use crate::db::DbPool;
use crate::state::AppState;

pub mod deps_impl;
pub mod registry;

pub use registry::{ActiveCallRegistry, ActiveGroupCallRegistry};

pub struct CallsAdapter {
    pub(super) state: Arc<AppState>,
    pub(super) app_handle: tauri::AppHandle,
    pub(super) pool: DbPool,
    pub(super) registry: Arc<dyn CallRegistry>,
    pub(super) group_registry: Arc<dyn GroupCallRegistry>,
}

impl CallsAdapter {
    /// Internal helper for emit_event: friend's display name, or a
    /// truncated short_pubkey if missing. Matches the pre-Phase-14
    /// `state_helpers::friend_display_name(...).unwrap_or_else(|| short_pubkey(...))`
    /// pattern for outbound UI events. The `friend_display_name` trait
    /// method (called by the crate handlers) returns the raw value so
    /// the crate's "if empty then short_pubkey(initiator_pubkey)"
    /// fallback path still works correctly for incoming-invite display.
    pub(super) fn display_name_with_fallback(&self, peer_pubkey_hex: &str) -> String {
        let raw = self
            .state
            .friends
            .read()
            .get(peer_pubkey_hex)
            .map(|f| f.display_name.clone())
            .unwrap_or_default();
        if raw.is_empty() {
            if peer_pubkey_hex.len() > 16 {
                format!("{}…", &peer_pubkey_hex[..16])
            } else {
                peer_pubkey_hex.to_string()
            }
        } else {
            raw
        }
    }

    #[must_use]
    pub fn new(state: Arc<AppState>, app_handle: tauri::AppHandle, pool: DbPool) -> Arc<Self> {
        // Phase 14.q — active_calls is already an `Arc<dyn CallRegistry>`
        // on AppState; just clone the Arc rather than wrapping the
        // underlying HashMap a second time.
        let registry: Arc<dyn CallRegistry> = Arc::clone(&state.active_calls);
        let group_registry: Arc<dyn GroupCallRegistry> =
            Arc::new(ActiveGroupCallRegistry::new(Arc::clone(&state.group_calls)));
        Arc::new(Self {
            state,
            app_handle,
            pool,
            registry,
            group_registry,
        })
    }
}

/// Public free-fn facade — dispatch an inbound group-call gossip
/// `MessagePayload` through the crate's group_handlers. Used by
/// `services::message_service` (the app_message dispatcher).
pub async fn handle_group_call_payload(
    app: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    sender_hex: &str,
    payload: rekindle_protocol::messaging::envelope::MessagePayload,
) {
    let adapter = CallsAdapter::new(state.clone(), app.clone(), pool.clone());
    rekindle_calls::signaling::group_handlers::handle_group_call_payload(
        adapter.as_ref(),
        sender_hex,
        payload,
    )
    .await;
}

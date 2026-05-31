//! Phase 13 — thin facade for outbound DM creation.
//!
//! All business logic moved to `rekindle_dm::session::start_dm` (the
//! initiator path: SMPL record allocation + ECDH MEK derive + persist
//! + watch peer subkey + `app_call` invite). This file builds a
//! `DmAdapter` and delegates.

use std::sync::Arc;

use crate::db::DbPool;
use crate::services::dm_adapter::DmAdapter;
use crate::state::AppState;
use crate::state_helpers;

/// Outbound 1:1 DM creation. Returns the new SMPL record key on
/// success; errors on accept-decline, network failure, identity-not-
/// loaded, etc.
pub async fn start_dm(
    state: &Arc<AppState>,
    pool: &DbPool,
    bob_public_key_hex: &str,
    alice_pseudonym: &str,
) -> Result<String, String> {
    let app_handle =
        state_helpers::app_handle(state).ok_or_else(|| "app handle unavailable".to_string())?;
    let adapter = DmAdapter::new(Arc::clone(state), app_handle, pool.clone());
    rekindle_dm::start_dm(&*adapter, bob_public_key_hex, alice_pseudonym)
        .await
        .map_err(|e| e.to_string())
}

//! Phase 13 — thin facade for DM invite-accept.
//!
//! All business logic moved to `rekindle_dm::session::accept_dm_invite`
//! (responder path: load invite, recover MEK, restore chain, open
//! record read-only, watch peer subkeys). This file builds a
//! `DmAdapter` and delegates.

use std::sync::Arc;

use crate::db::DbPool;
use crate::services::dm_adapter::DmAdapter;
use crate::state::AppState;
use crate::state_helpers;

/// Responder-side accept for an inbound DM invite.
pub async fn accept_dm_invite(
    state: &Arc<AppState>,
    pool: &DbPool,
    record_key: &str,
) -> Result<(), String> {
    let app_handle =
        state_helpers::app_handle(state).ok_or_else(|| "app handle unavailable".to_string())?;
    let adapter = DmAdapter::new(Arc::clone(state), app_handle, pool.clone());
    rekindle_dm::accept_dm_invite(&*adapter, record_key)
        .await
        .map_err(|e| e.to_string())
}

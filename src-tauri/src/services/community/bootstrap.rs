//! Phase 18.h.2 — thin facade.
//!
//! The bootstrap-response assembly (governance snapshot + wrapped owner
//! keypair + per-channel MEK wrap + recent message re-encrypt + capnp
//! envelope encode) now lives in
//! `rekindle_governance_runtime::bootstrap::build_bootstrap_response`.
//! This module constructs a `GovernanceAdapter` and delegates.

use std::sync::Arc;

use tauri::Manager;

use crate::state::AppState;

pub async fn build_bootstrap_response(
    state: &Arc<AppState>,
    community_id: &str,
    governance_key: &str,
    joiner_pseudonym_hex: &str,
) -> Result<Vec<u8>, String> {
    let app_handle = state
        .app_handle
        .read()
        .clone()
        .ok_or_else(|| "app handle unavailable".to_string())?;
    let pool = app_handle
        .try_state::<crate::db::DbPool>()
        .ok_or_else(|| "DbPool state missing".to_string())?
        .inner()
        .clone();
    let adapter = crate::services::governance_adapter::GovernanceAdapter::new(
        Arc::clone(state),
        app_handle,
        pool,
    );
    rekindle_governance_runtime::build_bootstrap_response(
        &adapter,
        community_id,
        governance_key,
        joiner_pseudonym_hex,
    )
    .await
    .map_err(|e| e.to_string())
}

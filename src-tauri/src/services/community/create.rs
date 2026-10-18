//! Phase 18.h.2 — thin facade.
//!
//! The full `create_community` pipeline (slot seed + 255 slot pubkeys +
//! 3 SMPL records + genesis governance entries + creator presence +
//! community MEK + CommunityState insert + background services) now
//! lives in `rekindle_governance_runtime::origin::create_community`.
//! This module constructs a `GovernanceAdapter` and delegates.

use std::sync::Arc;

use crate::state::AppState;

pub async fn create_community(
    state: &Arc<AppState>,
    name: &str,
    admission: rekindle_types::governance::AdmissionMode,
) -> Result<String, String> {
    let app_handle = state
        .app_handle
        .read()
        .clone()
        .ok_or_else(|| "app handle unavailable".to_string())?;
    let pool = state.db.current()?;
    let adapter = crate::services::governance_adapter::GovernanceAdapter::new(
        Arc::clone(state),
        app_handle,
        pool,
    );
    rekindle_governance_runtime::create_community(&adapter, name, admission)
        .await
        .map_err(|e| e.to_string())
}

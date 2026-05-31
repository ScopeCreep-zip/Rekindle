//! Phase 18.h.2 — thin facade.
//!
//! The full write_entry pipeline (signed payload + M9.5 conflict detect +
//! mesh broadcast + local CRDT merge + UI snapshot emit) now lives in
//! `rekindle_governance_runtime::apply::write_entry`. This module
//! constructs a `GovernanceAdapter` and delegates.

use std::sync::Arc;

use rekindle_types::governance::GovernanceEntry;
use tauri::Manager;

use crate::state::SharedState;

pub async fn write_entry(
    state: &SharedState,
    community_id: &str,
    entry: GovernanceEntry,
) -> Result<(), String> {
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
    rekindle_governance_runtime::write_entry(&adapter, community_id, entry)
        .await
        .map_err(|e| e.to_string())
}

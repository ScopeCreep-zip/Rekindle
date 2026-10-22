//! Phase 18.h.2 — thin facade.
//!
//! The full `create_community` pipeline (slot seed + 255 slot pubkeys +
//! 3 SMPL records + genesis governance entries + creator presence +
//! community MEK + CommunityState insert + background services) now
//! lives in `rekindle_governance_runtime::origin::create_community`.
//! This module runs it over the Host's `HostGovernance` (plan E3.1 B2f).

use std::sync::Arc;

use crate::state::AppState;

pub async fn create_community(
    state: &Arc<AppState>,
    name: &str,
    admission: rekindle_types::governance::AdmissionMode,
) -> Result<String, String> {
    let adapter = crate::state_helpers::host_governance(state)?;
    rekindle_governance_runtime::create_community(&adapter, name, admission)
        .await
        .map_err(|e| e.to_string())
}

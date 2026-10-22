//! Phase 18.h.2 — thin facade.
//!
//! The full write_entry pipeline (signed payload + M9.5 conflict detect +
//! mesh broadcast + local CRDT merge + UI snapshot emit) now lives in
//! `rekindle_governance_runtime::apply::write_entry`. This module
//! runs it over the Host's `HostGovernance` (plan E3.1 B2f).

use rekindle_types::governance::GovernanceEntry;

use crate::state::SharedState;

pub async fn write_entry(
    state: &SharedState,
    community_id: &str,
    entry: GovernanceEntry,
) -> Result<(), String> {
    let adapter = crate::state_helpers::host_governance(state)?;
    rekindle_governance_runtime::write_entry(&adapter, community_id, entry)
        .await
        .map_err(|e| e.to_string())
}

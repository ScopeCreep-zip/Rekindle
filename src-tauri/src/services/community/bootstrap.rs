//! Phase 18.h.2 — thin facade.
//!
//! The bootstrap-response assembly (governance snapshot + wrapped owner
//! keypair + per-channel MEK wrap + recent message re-encrypt + capnp
//! envelope encode) now lives in
//! `rekindle_governance_runtime::bootstrap::build_bootstrap_response`.
//! This module runs it over the Host's `HostGovernance` (plan E3.1 B2f).

use std::sync::Arc;

use crate::state::AppState;

pub async fn build_bootstrap_response(
    state: &Arc<AppState>,
    community_id: &str,
    governance_key: &str,
    joiner_pseudonym_hex: &str,
) -> Result<Vec<u8>, String> {
    let adapter = crate::state_helpers::host_governance(state)?;
    rekindle_governance_runtime::build_bootstrap_response(
        &adapter,
        community_id,
        governance_key,
        joiner_pseudonym_hex,
    )
    .await
    .map_err(|e| e.to_string())
}

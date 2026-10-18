//! The desktop's appends to the unlocked identity's audit chain
//! (`rekindle_node::host::audit`, plan D1), for its callers not yet ported.
//! Verifying the chain and its tamper alert are the Host's (plan E1.1).

use rekindle_audit::AuditKind;
use rekindle_db::Db;

use crate::state::AppState;

/// Append one entry to the unlocked identity's chain.
pub fn append(
    state: &AppState,
    pool: &Db,
    owner: &str,
    kind: AuditKind,
    payload: serde_json::Value,
) {
    rekindle_node::host::audit::append(
        &state.audit_chain,
        pool,
        &state.keystore,
        owner,
        kind,
        payload,
        state.clock.now_ms_i64(),
    );
}

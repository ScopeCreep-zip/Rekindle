//! Device-key acts in a community (ADR 0022, plan E3.2 S7d-keys K5):
//! retire one of our device keys, or co-sign a member's fork-answering
//! key back in, or enrol this device (`EnrolDevice` TrustChanging). All
//! are intent-gated (`RetireDevice` TrustChanging,
//! `ReadmitDevice` Consequential) before they reach here.

use rekindle_governance_runtime::{GovernanceRuntimeError, OpOutcome};
use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::id::PseudonymKey;
use rekindle_types::key_format;

use crate::daemon::dispatch::{adapter, state_error, DaemonContext};
use crate::daemon::DaemonState;

/// Enrol this device in `anchor` with a fresh device key (ADR 0022, lead
/// decision 63); answers the new key's hex.
pub(crate) async fn handle_device_enrol(
    ctx: &DaemonContext,
    state: DaemonState,
    anchor: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(anchor) {
        Ok(m) => m,
        Err(e) => return e,
    };
    match Box::pin(adapter(ctx).enrol_device(&membership.anchor)).await {
        Ok((outcome, key)) => answer("enrolled", &hex::encode(key), Ok(outcome)),
        Err(e) => answer("enrolled", "", Err(e)),
    }
}

/// Retire our device key `device_key` (hex) in `anchor`.
pub(crate) async fn handle_device_retire(
    ctx: &DaemonContext,
    state: DaemonState,
    anchor: &str,
    device_key: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(anchor) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let key = match key_format::public_key_hex(device_key) {
        Ok(k) => k.to_bytes(),
        Err(e) => return IpcResponse::error(400, format!("invalid device key: {e}")),
    };
    let written = Box::pin(adapter(ctx).retire_device(&membership.anchor, key)).await;
    answer("retired", device_key, written)
}

/// Co-sign the re-admission of `member`'s device key `device_key` (both
/// hex) in `anchor`.
pub(crate) async fn handle_device_readmit(
    ctx: &DaemonContext,
    state: DaemonState,
    anchor: &str,
    member: &str,
    device_key: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let membership = match ctx.resolve_community(anchor) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let (member, key) = match (
        key_format::pseudonym_hex(member),
        key_format::public_key_hex(device_key),
    ) {
        (Ok(m), Ok(k)) => (PseudonymKey(m.to_bytes()), k.to_bytes()),
        (Err(e), _) | (_, Err(e)) => return IpcResponse::error(400, format!("invalid key: {e}")),
    };
    let written = Box::pin(adapter(ctx).readmit_device(&membership.anchor, member, key)).await;
    answer("readmitted", device_key, written)
}

fn answer(
    field: &str,
    device_key: &str,
    written: Result<OpOutcome, GovernanceRuntimeError>,
) -> IpcResponse {
    match written {
        Ok(outcome) => IpcResponse::ok(&serde_json::json!({
            field: device_key,
            "pending": outcome == OpOutcome::Pending,
            "heldOpId": outcome.held_op_id(),
        })),
        Err(e @ GovernanceRuntimeError::DeviceActRefused(_)) => {
            IpcResponse::error(409, e.to_string())
        }
        Err(e @ GovernanceRuntimeError::PermissionDenied) => IpcResponse::error(403, e.to_string()),
        Err(e) => IpcResponse::error(500, format!("device act failed: {e}")),
    }
}

//! Join-request moderation: approve, reject, pending list.

use crate::daemon::DaemonState;
use crate::ipc::protocol::IpcResponse;

use crate::daemon::dispatch::{state_error, DaemonContext};

fn adapter(ctx: &DaemonContext) -> crate::daemon::governance_adapter::DaemonGovernanceAdapter<'_> {
    crate::daemon::governance_adapter::DaemonGovernanceAdapter::new(ctx)
}

/// Admit a pending member.
///
/// Writes a `MemberApproved` governance entry and stops. It no longer
/// reads the moderation queue, computes `max(subkey_index)+1`, or writes
/// the member index — that was the coordinator. The member claims its
/// own registry slot during join.
///
/// It also no longer wraps MEKs into the registry vault: the joiner gets
/// the key from the invite, or by `RequestMek` gossip if the invite is
/// stale (join step 10), both already implemented on this track.
///
/// This records a decision; it does not grant access. Every peer
/// validates it independently — same as Jami, where a join commit
/// missing its `/invited` precondition is rejected by every peer rather
/// than blocked at the source.
pub(crate) async fn handle_approve(
    ctx: &DaemonContext,
    state: DaemonState,
    governance_key: &str,
    member_pseudonym: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    if let Err(e) = ctx.resolve_community(governance_key) {
        return e;
    }
    match rekindle_governance_runtime::admission::approve_member(
        &adapter(ctx),
        governance_key,
        member_pseudonym,
    )
    .await
    {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "approved": member_pseudonym })),
        Err(e) => IpcResponse::error(500, format!("approve failed: {e}")),
    }
}

/// Refuse a pending member — writes `MemberRejected`.
pub(crate) async fn handle_reject(
    ctx: &DaemonContext,
    state: DaemonState,
    governance_key: &str,
    member_pseudonym: &str,
    reason: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    if let Err(e) = ctx.resolve_community(governance_key) {
        return e;
    }
    // Empty string from the CLI means "no reason given"; keep that
    // distinct from an explicit empty reason on the wire.
    let reason = (!reason.is_empty()).then_some(reason);
    match rekindle_governance_runtime::admission::reject_member(
        &adapter(ctx),
        governance_key,
        member_pseudonym,
        reason,
    )
    .await
    {
        Ok(()) => {
            IpcResponse::ok(&serde_json::json!({ "rejected": member_pseudonym, "reason": reason }))
        }
        Err(e) => IpcResponse::error(500, format!("reject failed: {e}")),
    }
}

/// Everyone awaiting a decision, read from merged governance state
/// rather than registry subkey 5.
///
/// Under `AdmissionMode::Open` this is always empty — a joiner claims a
/// slot without asking — so an empty list does not mean "nobody wants
/// in". The mode is returned alongside so a frontend can tell the two
/// apart.
pub(crate) fn handle_pending_members(
    ctx: &DaemonContext,
    state: DaemonState,
    governance_key: &str,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    if let Err(e) = ctx.resolve_community(governance_key) {
        return e;
    }
    let adapter = adapter(ctx);
    let pending: Vec<serde_json::Value> =
        rekindle_governance_runtime::admission::pending_members(&adapter, governance_key)
            .into_iter()
            .map(|p| {
                serde_json::json!({
                    "pseudonymHex": p.pseudonym_hex,
                    "displayName": p.display_name,
                    "lamport": p.lamport,
                })
            })
            .collect();
    let mode =
        match rekindle_governance_runtime::admission::admission_mode(&adapter, governance_key) {
            rekindle_types::governance::AdmissionMode::Open => "open",
            rekindle_types::governance::AdmissionMode::ApprovalRequired => "approvalRequired",
        };
    IpcResponse::ok(&serde_json::json!({ "mode": mode, "pending": pending }))
}

//! The selection of a group-DM request (plan E2.6): the group by its
//! creator's name, the members each step tells, and a send's escalation
//! on a friend member's key change.

use rekindle_idempotency::intent_gate::Selection;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_transport::operations::group_dm;
use rekindle_types::intent::{IntentKind, Notifies, Reversibility, Subject};

use super::unavailable;
use crate::daemon::dispatch::DaemonContext;
use crate::host::adapters::friends::HostFriends;

fn count(n: usize) -> Notifies {
    Notifies::Members(u32::try_from(n).unwrap_or(u32::MAX))
}

/// Fill `s` for a group-DM request.
///
/// # Errors
/// The groups cannot be read, or no group or invite has that id.
pub(super) async fn selection(
    ctx: &DaemonContext,
    request: &IpcRequest,
    s: &mut Selection,
) -> Result<(), IpcResponse> {
    let group_id = match request {
        IpcRequest::GroupDmCreate { name, members } => {
            s.subject = Subject::Named { name: name.clone() };
            s.notifies = count(members.len());
            s.reversibility = Reversibility::NeedsOthers;
            return Ok(());
        }
        IpcRequest::GroupDmAccept { group_id }
        | IpcRequest::GroupDmDecline { group_id }
        | IpcRequest::GroupDmLeave { group_id }
        | IpcRequest::GroupDmSend { group_id, .. } => group_id,
        other => unreachable!("{} is not a group-DM write", other.name()),
    };
    if ctx.require_owner().is_err() {
        return Ok(());
    }
    let view = group_dm::find(&HostFriends::of(ctx), group_id)
        .await
        .map_err(|e| unavailable("the group", e))?
        .ok_or_else(|| IpcResponse::error(404, "no such group or invite"))?;
    let others = view.members.len().saturating_sub(1);
    s.subject = Subject::Named { name: view.name };
    match request {
        IpcRequest::GroupDmAccept { .. } => {
            s.notifies = count(others);
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::LeaveGroupDm);
        }
        IpcRequest::GroupDmDecline { .. } => {
            s.notifies = count(1);
            s.reversibility = Reversibility::NeedsOthers;
        }
        IpcRequest::GroupDmSend { .. } => {
            // Reviewed as a direct send: escalated on each friend
            // member's unacknowledged verified key change (R-4).
            s.notifies = count(others);
            s.escalations.extend(
                group_dm::key_changes(&HostFriends::of(ctx), group_id)
                    .await
                    .map_err(|e| unavailable("the members' key verification", e))?,
            );
        }
        _ => {
            s.notifies = count(others);
            s.reversibility = Reversibility::NeedsOthers;
        }
    }
    Ok(())
}

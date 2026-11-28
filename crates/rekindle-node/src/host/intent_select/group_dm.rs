//! The selection of a group-DM request (plan E2.6): the group by its
//! creator's name, the members each step tells, and a send's escalation
//! on a friend member's key change.

use rekindle_idempotency::intent_gate::Selection;
use rekindle_ipc::protocol::{GroupDmRequest, IpcRequest, IpcResponse, LobbyRequest};
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
        IpcRequest::GroupDm(GroupDmRequest::Create { name, members, .. }) => {
            s.subject = Subject::Named { name: name.clone() };
            s.notifies = count(members.len());
            s.reversibility = Reversibility::NeedsOthers;
            return Ok(());
        }
        IpcRequest::DisappearingSet { conversation, .. } if conversation.len() != 32 => {
            // Our DM with a friend: they are told and shown the notice.
            if ctx.require_owner().is_ok() {
                s.subject = super::friends::person(ctx, conversation).await?;
            }
            s.notifies = count(1);
            s.reversibility = Reversibility::NeedsOthers;
            return Ok(());
        }
        IpcRequest::DisappearingSet {
            conversation: group_id,
            ..
        }
        | IpcRequest::GroupDm(
            GroupDmRequest::Accept { group_id }
            | GroupDmRequest::Decline { group_id }
            | GroupDmRequest::Leave { group_id }
            | GroupDmRequest::Rename { group_id, .. }
            | GroupDmRequest::Invite { group_id, .. }
            | GroupDmRequest::Send { group_id, .. },
        )
        | IpcRequest::Lobby(
            LobbyRequest::Host { group_id, .. }
            | LobbyRequest::Join { group_id, .. }
            | LobbyRequest::Leave { group_id, .. }
            | LobbyRequest::Close { group_id, .. }
            | LobbyRequest::Launch { group_id, .. },
        ) => group_id,
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
    s.subject = Subject::Named {
        name: view.name.clone(),
    };
    match request {
        IpcRequest::GroupDm(GroupDmRequest::Accept { .. }) => {
            s.notifies = count(others);
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::LeaveGroupDm);
        }
        IpcRequest::GroupDm(GroupDmRequest::Rename { .. }) => {
            s.notifies = count(others);
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::RenameGroupDm);
        }
        IpcRequest::GroupDm(GroupDmRequest::Invite { members, .. }) => {
            // Every member it had, and each newcomer, is told.
            s.notifies = count(others + members.len());
            s.reversibility = Reversibility::NeedsOthers;
        }
        IpcRequest::Lobby(LobbyRequest::Host { .. }) => {
            s.notifies = count(others);
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::CloseGameLobby);
        }
        IpcRequest::Lobby(LobbyRequest::Join { .. }) => {
            s.notifies = count(others);
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::LeaveGameLobby);
        }
        IpcRequest::Lobby(LobbyRequest::Launch { server, .. }) => {
            // TrustChanging (`LaunchGameLobby`): the review names the
            // address every member receives.
            let server = super::super::lobby::launch_server(ctx, server.as_ref())?;
            s.subject = Subject::Named {
                name: format!("{} at {server}", view.name),
            };
            s.notifies = count(others);
            s.reversibility = Reversibility::NeedsOthers;
        }
        IpcRequest::GroupDm(GroupDmRequest::Decline { .. }) => {
            s.notifies = count(1);
            s.reversibility = Reversibility::NeedsOthers;
        }
        IpcRequest::GroupDm(GroupDmRequest::Send { .. }) => {
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

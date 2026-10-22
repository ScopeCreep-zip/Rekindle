//! A game-server change's review (plan E3.1 H1): the server by its label,
//! reaching the community's members.

use rekindle_idempotency::intent_gate::Selection;
use rekindle_ipc::protocol::ActivityRequest;
use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::intent::{IntentKind, Notifies, Reversibility, Subject};

use crate::daemon::dispatch::DaemonContext;

/// Fill `s` for a game-server add or removal.
///
/// # Errors
/// The community is not joined, or the server to remove is not stored.
pub(super) async fn selection(
    ctx: &DaemonContext,
    request: &ActivityRequest,
    s: &mut Selection,
) -> Result<(), IpcResponse> {
    match request {
        ActivityRequest::GameServerAdd {
            community, label, ..
        } => {
            let id = ctx.resolve_community(community)?.governance_key;
            s.subject = Subject::Named {
                name: label.clone(),
            };
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::RemoveGameServer);
            s.notifies = members(ctx, &id);
        }
        ActivityRequest::GameServerRemove {
            community,
            server_id,
        } => {
            let id = ctx.resolve_community(community)?.governance_key;
            let governance = crate::daemon::dispatch::adapter(ctx);
            let label = governance
                .game_server_label(&id, server_id)
                .await
                .map_err(|e| IpcResponse::error(500, format!("game server: {e}")))?
                .ok_or_else(|| IpcResponse::error(404, "no such game server"))?;
            s.subject = Subject::Named { name: label };
            s.notifies = members(ctx, &id);
        }
        _ => {}
    }
    Ok(())
}

/// The members a community action reaches.
fn members(ctx: &DaemonContext, community_id: &str) -> Notifies {
    Notifies::Members(u32::try_from(ctx.communities.member_count(community_id)).unwrap_or(u32::MAX))
}

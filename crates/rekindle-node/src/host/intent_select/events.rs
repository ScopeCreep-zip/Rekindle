//! An event change's review (plan E3.1 E4): the event by its title,
//! reaching the community's members.

use rekindle_idempotency::intent_gate::Selection;
use rekindle_ipc::protocol::ActivityRequest;
use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::intent::{IntentKind, Notifies, Reversibility, Subject};

use crate::daemon::dispatch::DaemonContext;

/// Fill `s` for an event create, edit, cancel, deletion or answer.
///
/// # Errors
/// The community is not joined, or the event named is not known.
pub(super) async fn selection(
    ctx: &DaemonContext,
    request: &ActivityRequest,
    s: &mut Selection,
) -> Result<(), IpcResponse> {
    let (community, event_id) = match request {
        ActivityRequest::EventCreate { community, event } => {
            let id = ctx.resolve_community(community)?.governance_key;
            s.subject = Subject::Named {
                name: event.title.clone(),
            };
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::DeleteEvent);
            s.notifies = members(ctx, &id);
            return Ok(());
        }
        ActivityRequest::EventEdit {
            community,
            event_id,
            ..
        }
        | ActivityRequest::EventCancel {
            community,
            event_id,
        }
        | ActivityRequest::EventDelete {
            community,
            event_id,
        }
        | ActivityRequest::EventRsvp {
            community,
            event_id,
            ..
        } => (community, event_id),
        _ => return Ok(()),
    };
    let id = ctx.resolve_community(community)?.governance_key;
    let title = crate::daemon::dispatch::adapter(ctx)
        .events(&id)
        .await
        .map_err(|e| IpcResponse::error(500, format!("events: {e}")))?
        .into_iter()
        .find(|e| e.id == *event_id)
        .map(|e| e.title)
        .ok_or_else(|| IpcResponse::error(404, "no such event"))?;
    s.subject = Subject::Named { name: title };
    s.notifies = members(ctx, &id);
    Ok(())
}

/// The members a community action reaches.
fn members(ctx: &DaemonContext, community_id: &str) -> Notifies {
    Notifies::Members(u32::try_from(ctx.communities.member_count(community_id)).unwrap_or(u32::MAX))
}

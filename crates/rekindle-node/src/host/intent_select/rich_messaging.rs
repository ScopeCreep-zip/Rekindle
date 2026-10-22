//! The selection of a reaction, thread or poll write (plan E3.1 E3): the
//! channel it is in — a thread's parent channel — and the members it
//! reaches.

use rekindle_idempotency::intent_gate::Selection;
use rekindle_ipc::protocol::IpcResponse;
use rekindle_ipc::protocol::RichRequest;
use rekindle_types::id::ThreadId;
use rekindle_types::intent::{IntentKind, Reversibility, Subject};

use super::community::{channel_name, community, members};
use crate::daemon::dispatch::DaemonContext;

/// The parent channel of `thread_id` in `community_id`'s governance.
fn thread_channel(
    ctx: &DaemonContext,
    community_id: &str,
    thread_id: &str,
) -> Result<String, IpcResponse> {
    let not_found = || IpcResponse::error(404, format!("thread '{thread_id}' not found"));
    let id: [u8; 16] = (hex::decode(thread_id).ok())
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(not_found)?;
    let gov = ctx
        .communities
        .governance_state(community_id)
        .ok_or_else(not_found)?;
    let thread = gov.threads.get(&ThreadId(id)).ok_or_else(not_found)?;
    Ok(hex::encode(thread.parent_channel_id.0))
}

/// Fill `s` for a reaction, thread or poll write.
///
/// # Errors
/// The community is not joined, or the thread is not in its governance.
pub(super) fn selection(
    ctx: &DaemonContext,
    request: &RichRequest,
    s: &mut Selection,
) -> Result<(), IpcResponse> {
    let (target, channel) = match request {
        RichRequest::ReactionAdd {
            community,
            channel_id,
            ..
        }
        | RichRequest::ReactionRemove {
            community,
            channel_id,
            ..
        }
        | RichRequest::ThreadCreate {
            community,
            channel_id,
            ..
        }
        | RichRequest::PollCreate {
            community,
            channel_id,
            ..
        }
        | RichRequest::PollVote {
            community,
            channel_id,
            ..
        }
        | RichRequest::PollClose {
            community,
            channel_id,
            ..
        } => (community.clone(), channel_id.clone()),
        RichRequest::ThreadArchive {
            community: c,
            thread_id,
        }
        | RichRequest::ThreadUnarchive {
            community: c,
            thread_id,
        }
        | RichRequest::ThreadSend {
            community: c,
            thread_id,
            ..
        } => {
            let (id, _) = community(ctx, c)?;
            (c.clone(), thread_channel(ctx, &id, thread_id)?)
        }
        other => {
            return Err(IpcResponse::error(
                500,
                format!("{} is not a reaction, thread or poll write", other.name()),
            ))
        }
    };
    let (id, community_name) = community(ctx, &target)?;
    s.subject = Subject::Channel {
        name: channel_name(ctx, &id, &channel),
        community_name,
        id: channel,
    };
    s.notifies = members(ctx, &id);
    match request {
        RichRequest::ReactionAdd { .. } => {
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::RemoveReaction);
        }
        RichRequest::ReactionRemove { .. } => {
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::AddReaction);
        }
        RichRequest::ThreadArchive { .. } => {
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::UnarchiveThread);
        }
        _ => {}
    }
    Ok(())
}

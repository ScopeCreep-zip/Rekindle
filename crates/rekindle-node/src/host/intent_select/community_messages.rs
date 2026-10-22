//! The selection of a channel message forward or a moderator's delete
//! (plan E3.1 E2d): the channel it lands in or deletes from, the members it
//! reaches, and for a forward the E4 `CrossAudience` when the destination's
//! readers differ from the source's (`intent_facts::forward_escalations`
//! over `channel_members`).

use rekindle_governance::intent_facts::{forward_escalations, ChannelAudience};
use rekindle_idempotency::intent_gate::Selection;
use rekindle_ipc::protocol::ChannelMessageRequest;
use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::intent::Subject;

use super::community::{channel_name, community, members, roster};
use super::unavailable;
use crate::daemon::dispatch::channel::resolve_channel;
use crate::daemon::dispatch::DaemonContext;

/// Fill `s` for a forward, an admin delete or a bulk delete.
///
/// # Errors
/// A community is not joined, a channel is unknown, or a governance is not
/// loaded.
pub(super) fn selection(
    ctx: &DaemonContext,
    request: &ChannelMessageRequest,
    s: &mut Selection,
) -> Result<(), IpcResponse> {
    match request {
        ChannelMessageRequest::ChannelMessageForward {
            source_community,
            source_channel,
            dest_community,
            dest_channel,
            ..
        } => {
            let (from_id, _) = community(ctx, source_community)?;
            let (to_id, to_name) = community(ctx, dest_community)?;
            s.escalations =
                cross_audience(ctx, (&from_id, source_channel), (&to_id, dest_channel))?;
            s.subject = Subject::Channel {
                name: channel_name(ctx, &to_id, dest_channel),
                community_name: to_name,
                id: dest_channel.clone(),
            };
            s.notifies = members(ctx, &to_id);
        }
        ChannelMessageRequest::ChannelMessageAdminDelete {
            community: target,
            channel,
            ..
        }
        | ChannelMessageRequest::ChannelMessageBulkDelete {
            community: target,
            channel,
            ..
        } => {
            let (id, community_name) = community(ctx, target)?;
            s.subject = Subject::Channel {
                name: channel_name(ctx, &id, channel),
                community_name,
                id: channel.clone(),
            };
            s.notifies = members(ctx, &id);
        }
        _ => {}
    }
    Ok(())
}

/// E4 for a forward from `(community, channel)` `from` into `to`.
fn cross_audience(
    ctx: &DaemonContext,
    from: (&str, &str),
    to: (&str, &str),
) -> Result<Vec<rekindle_types::intent::Escalation>, IpcResponse> {
    let side = |(community_id, channel): (&str, &str)| {
        let state = ctx
            .communities
            .governance_state(community_id)
            .ok_or_else(|| unavailable("the community's governance", "not loaded"))?;
        let (id, _) = resolve_channel(ctx, community_id, channel)
            .ok_or_else(|| IpcResponse::error(404, format!("channel '{channel}' not found")))?;
        Ok::<_, IpcResponse>((state, id, roster(ctx, community_id)))
    };
    let (from_state, from_channel, from_members) = side(from)?;
    let (to_state, to_channel, to_members) = side(to)?;
    Ok(forward_escalations(
        ChannelAudience {
            state: &from_state,
            channel: &from_channel,
            members: &from_members,
        },
        ChannelAudience {
            state: &to_state,
            channel: &to_channel,
            members: &to_members,
        },
    ))
}

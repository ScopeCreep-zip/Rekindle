//! The selection of a community, channel, role or moderation request
//! (plans D3.3, E3.1): the community, channel, role or member it is about,
//! and the members it reaches.

mod audit;
mod channels;
mod expressions;
mod lifecycle;
mod members;
mod messages;
mod pins;

use rekindle_idempotency::intent_gate::Selection;
use rekindle_ipc::protocol::{
    AuditRequest, ChannelMessageRequest, ChannelRequest, ExpressionRequest, LifecycleRequest,
    MemberRequest, PinRequest, SettingsRequest,
};
use rekindle_ipc::protocol::{CommunityRequest, IpcResponse};
use rekindle_types::intent::{Escalation, Fact, IntentKind, Notifies, Reversibility, Subject};

use super::unavailable;
use crate::daemon::dispatch::DaemonContext;

/// Fill `s` for a community, channel, role or moderation request.
///
/// # Errors
/// The community is not joined, or its governance is not loaded.
pub(super) async fn selection(
    ctx: &DaemonContext,
    request: &CommunityRequest,
    s: &mut Selection,
) -> Result<(), IpcResponse> {
    match request {
        CommunityRequest::Lifecycle(request) => lifecycle::selection(ctx, request, s),
        CommunityRequest::Channels(request) => channels::selection(ctx, request, s).await,
        CommunityRequest::Messages(request) => messages::selection(ctx, request, s),
        CommunityRequest::Audit(request) => audit::selection(ctx, request, s),
        CommunityRequest::Members(request) => members::selection(ctx, request, s),
        CommunityRequest::Pins(request) => pins::selection(ctx, request, s),
        CommunityRequest::Expressions(request) => expressions::selection(ctx, request, s),
        CommunityRequest::Settings(_)
        | CommunityRequest::Unread(_)
        | CommunityRequest::Activities(_)
        | CommunityRequest::Rich(_) => Ok(()),
    }
}

/// Fill `s` for a role grant or a moderation request on one member.
fn member_selection(
    ctx: &DaemonContext,
    request: &MemberRequest,
    s: &mut Selection,
) -> Result<(), IpcResponse> {
    match request {
        MemberRequest::RoleAssign {
            community: target,
            member_pseudonym,
            ..
        }
        | MemberRequest::RoleUnassign {
            community: target,
            member_pseudonym,
            ..
        }
        | MemberRequest::Kick {
            community: target,
            target_pseudonym: member_pseudonym,
        }
        | MemberRequest::Ban {
            community: target,
            target_pseudonym: member_pseudonym,
            ..
        }
        | MemberRequest::Unban {
            community: target,
            target_pseudonym: member_pseudonym,
        }
        | MemberRequest::Timeout {
            community: target,
            target_pseudonym: member_pseudonym,
            ..
        }
        | MemberRequest::TimeoutRemove {
            community: target,
            target_pseudonym: member_pseudonym,
        }
        | MemberRequest::MemberBlock {
            community: target,
            target_pseudonym: member_pseudonym,
        }
        | MemberRequest::MemberUnblock {
            community: target,
            target_pseudonym: member_pseudonym,
        } => {
            let (id, _) = community(ctx, target)?;
            s.subject = Subject::Named {
                name: member_pseudonym.clone(),
            };
            s.reversibility = match request {
                MemberRequest::RoleAssign { .. } => {
                    Reversibility::ReversibleByInverse(IntentKind::UnassignRole)
                }
                MemberRequest::RoleUnassign { .. } => {
                    Reversibility::ReversibleByInverse(IntentKind::AssignRole)
                }
                MemberRequest::Ban { .. } => {
                    Reversibility::ReversibleByInverse(IntentKind::UnbanMember)
                }
                MemberRequest::Unban { .. } => {
                    Reversibility::ReversibleByInverse(IntentKind::BanMember)
                }
                MemberRequest::Timeout { .. } => {
                    Reversibility::ReversibleByInverse(IntentKind::RemoveTimeout)
                }
                MemberRequest::TimeoutRemove { .. } => {
                    Reversibility::ReversibleByInverse(IntentKind::TimeoutMember)
                }
                MemberRequest::MemberBlock { .. } => {
                    Reversibility::ReversibleByInverse(IntentKind::UnblockUser)
                }
                MemberRequest::MemberUnblock { .. } => {
                    Reversibility::ReversibleByInverse(IntentKind::BlockUser)
                }
                MemberRequest::Kick { .. } => {
                    s.facts.push(Fact::RejoinNeedsInvite);
                    Reversibility::NeedsOthers
                }
                _ => Reversibility::Undoable,
            };
            // A block is ours alone: it reaches no member.
            if !matches!(
                request,
                MemberRequest::MemberBlock { .. } | MemberRequest::MemberUnblock { .. }
            ) {
                s.notifies = members(ctx, &id);
            }
        }
        _ => {}
    }
    Ok(())
}

/// A community by governance key or name: `(id, name)`.
pub(super) fn community(
    ctx: &DaemonContext,
    target: &str,
) -> Result<(String, String), IpcResponse> {
    ctx.resolve_community(target)
        .map(|m| (m.governance_key, m.community_name))
}

/// The members a community action reaches (its known members, the
/// validated roster in the daemon).
pub(super) fn members(ctx: &DaemonContext, community_id: &str) -> Notifies {
    Notifies::Members(u32::try_from(ctx.communities.member_count(community_id)).unwrap_or(u32::MAX))
}

pub(super) fn channel_name(ctx: &DaemonContext, community_id: &str, channel_id: &str) -> String {
    let parsed = hex::decode(channel_id)
        .ok()
        .and_then(|bytes| <[u8; 16]>::try_from(bytes).ok())
        .map(rekindle_types::id::ChannelId);
    parsed
        .and_then(|id| {
            ctx.communities
                .governance_state(community_id)
                .and_then(|state| state.channels.get(&id).map(|c| c.name.clone()))
        })
        .unwrap_or_else(|| channel_id.to_string())
}

/// A role's name and how many hold it.
fn role(
    ctx: &DaemonContext,
    community_id: &str,
    role_id: u32,
) -> Result<(String, u32), IpcResponse> {
    let state = ctx
        .communities
        .governance_state(community_id)
        .ok_or_else(|| unavailable("the community's governance", "not loaded"))?;
    let id = rekindle_types::id::RoleId::from_legacy_u32(role_id);
    let name = state
        .roles
        .get(&id)
        .map_or_else(|| role_id.to_string(), |r| r.name.clone());
    Ok((
        name,
        rekindle_governance::intent_facts::role_holders(&state, &id),
    ))
}

/// E6 for leaving: the last admin, or the only member.
fn leave_escalations(
    ctx: &DaemonContext,
    community_id: &str,
) -> Result<Vec<Escalation>, IpcResponse> {
    let state = ctx
        .communities
        .governance_state(community_id)
        .ok_or_else(|| unavailable("the community's governance", "not loaded"))?;
    let me = pseudonym(&ctx.my_pseudonym(community_id))
        .ok_or_else(|| unavailable("our pseudonym", "not in the community"))?;
    let members: Vec<_> = ctx
        .communities
        .known_members(community_id)
        .iter()
        .filter_map(|key| pseudonym(key))
        .collect();
    Ok(rekindle_governance::intent_facts::leave_escalations(
        &state,
        &me,
        &members,
        ctx.clock.now_secs(),
    ))
}

/// E7 for deleting `channel` (plan E3.1): the messages we store for it
/// (`repo::intent_facts::channel_messages`) and the members who read it
/// (`intent_facts::channel_members` over the registry's members).
async fn channel_delete_escalations(
    ctx: &DaemonContext,
    community_id: &str,
    channel: &str,
) -> Result<Vec<Escalation>, IpcResponse> {
    let state = ctx
        .communities
        .governance_state(community_id)
        .ok_or_else(|| unavailable("the community's governance", "not loaded"))?;
    let Some((id, hex_id)) =
        crate::daemon::dispatch::channel::resolve_channel(ctx, community_id, channel)
    else {
        return Err(IpcResponse::error(
            404,
            format!("channel '{channel}' not found"),
        ));
    };
    let members = roster(ctx, community_id);
    let (owner, db) = (ctx.require_owner()?, ctx.require_db()?);
    let messages = db
        .call(move |conn| rekindle_db::repo::intent_facts::channel_messages(conn, &owner, &hex_id))
        .await
        .map_err(|e| unavailable("the channel's messages", e))?;
    Ok(
        rekindle_governance::intent_facts::channel_delete_escalations(
            &state, &id, &members, messages,
        ),
    )
}

/// The community's members, from the registry, and ourselves: the members
/// a channel's read roster is drawn from.
pub(super) fn roster(
    ctx: &DaemonContext,
    community_id: &str,
) -> Vec<rekindle_types::id::PseudonymKey> {
    let mut members: Vec<_> = ctx
        .communities
        .known_members(community_id)
        .iter()
        .filter_map(|key| pseudonym(key))
        .collect();
    members.extend(pseudonym(&ctx.my_pseudonym(community_id)));
    members
}

pub(super) fn pseudonym(hex_key: &str) -> Option<rekindle_types::id::PseudonymKey> {
    hex::decode(hex_key)
        .ok()
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .map(rekindle_types::id::PseudonymKey)
}

/// Fill `s` for a community settings request: its profile, pictures,
/// policy, onboarding flow, welcome screen and our onboarding.
///
/// # Errors
/// The community is not joined.
pub(super) fn settings_selection(
    ctx: &DaemonContext,
    request: &SettingsRequest,
    s: &mut Selection,
) -> Result<(), IpcResponse> {
    match request {
        SettingsRequest::CommunityUpdateProfile {
            community: target, ..
        }
        | SettingsRequest::CommunitySetAvatar {
            community: target, ..
        }
        | SettingsRequest::CommunitySetBanner {
            community: target, ..
        } => {
            let (id, name) = community(ctx, target)?;
            s.reversibility = Reversibility::Undoable;
            s.subject = Subject::Community { id, name };
        }
        SettingsRequest::CommunityPolicySet {
            community: target, ..
        } => {
            let (id, name) = community(ctx, target)?;
            s.reversibility = Reversibility::Undoable;
            s.notifies = members(ctx, &id);
            s.subject = Subject::Community { id, name };
        }
        SettingsRequest::OnboardingConfigSet {
            community: target, ..
        }
        | SettingsRequest::WelcomeScreenSet {
            community: target, ..
        } => {
            let (id, name) = community(ctx, target)?;
            s.notifies = members(ctx, &id);
            s.subject = Subject::Community { id, name };
        }
        SettingsRequest::OnboardingSubmit {
            community: target, ..
        }
        | SettingsRequest::OnboardingComplete { community: target } => {
            let (id, name) = community(ctx, target)?;
            s.subject = Subject::Community { id, name };
        }
        _ => {}
    }
    Ok(())
}

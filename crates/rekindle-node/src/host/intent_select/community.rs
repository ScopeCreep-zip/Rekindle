//! The selection of a community, channel, role or moderation request
//! (plans D3.3, E3.1): the community, channel, role or member it is about,
//! and the members it reaches.

use rekindle_idempotency::intent_gate::Selection;
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
        CommunityRequest::CommunityCreate { name, .. } => {
            s.subject = Subject::Community {
                id: String::new(),
                name: name.clone(),
            };
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::LeaveCommunity);
        }
        CommunityRequest::CommunityJoin { invite } => {
            // The invite names a record key, not a community we know: the
            // review shows its fingerprint (`deep_links` in the shell).
            let link = rekindle_types::invite::InviteLink::parse(invite)
                .map_err(|e| IpcResponse::error(400, format!("community invite: {e}")))?;
            let fingerprint =
                rekindle_crypto::identity::record_key_fingerprint(link.governance_key.as_str())
                    .ok_or_else(|| {
                        IpcResponse::error(400, "community invite key is not a record key")
                    })?;
            s.subject = Subject::Named { name: fingerprint };
            s.facts.push(Fact::PresenceVisible);
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::LeaveCommunity);
            s.notifies = Notifies::Subject;
        }
        CommunityRequest::CommunityLeave { governance_key } => {
            let (id, name) = community(ctx, governance_key)?;
            s.escalations = leave_escalations(ctx, &id)?;
            s.facts.push(Fact::RejoinNeedsInvite);
            s.reversibility = Reversibility::NeedsOthers;
            s.notifies = members(ctx, &id);
            s.subject = Subject::Community { id, name };
        }
        CommunityRequest::CommunityUpdateInfo { governance_key, .. } => {
            let (id, name) = community(ctx, governance_key)?;
            s.reversibility = Reversibility::Undoable;
            s.notifies = members(ctx, &id);
            s.subject = Subject::Community { id, name };
        }
        CommunityRequest::CommunityUpdateProfile {
            community: target, ..
        }
        | CommunityRequest::CommunitySetAvatar {
            community: target, ..
        }
        | CommunityRequest::CommunitySetBanner {
            community: target, ..
        } => {
            let (id, name) = community(ctx, target)?;
            s.reversibility = Reversibility::Undoable;
            s.subject = Subject::Community { id, name };
        }
        CommunityRequest::CommunityPolicySet {
            community: target, ..
        } => {
            let (id, name) = community(ctx, target)?;
            s.reversibility = Reversibility::Undoable;
            s.notifies = members(ctx, &id);
            s.subject = Subject::Community { id, name };
        }
        CommunityRequest::CommunityApprove {
            governance_key,
            member_pseudonym,
        }
        | CommunityRequest::CommunityReject {
            governance_key,
            member_pseudonym,
            ..
        }
        | CommunityRequest::CommunityTransferOwnership {
            governance_key,
            new_owner_pseudonym: member_pseudonym,
        } => {
            let (id, _) = community(ctx, governance_key)?;
            s.subject = Subject::Named {
                name: member_pseudonym.clone(),
            };
            s.notifies = members(ctx, &id);
        }
        CommunityRequest::ChannelCreate {
            community: target,
            name,
            ..
        } => {
            let (id, community_name) = community(ctx, target)?;
            s.subject = Subject::Channel {
                community_name,
                id: String::new(),
                name: name.clone(),
            };
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::DeleteChannel);
            s.notifies = members(ctx, &id);
        }
        CommunityRequest::ChannelDelete {
            community: target,
            channel_id: channel,
        }
        | CommunityRequest::ChannelUpdate {
            community: target,
            channel_id: channel,
            ..
        }
        | CommunityRequest::ChannelSend {
            community: target,
            channel,
            ..
        }
        | CommunityRequest::ChannelRename {
            community: target,
            channel_id: channel,
            ..
        }
        | CommunityRequest::ChannelMove {
            community: target,
            channel_id: channel,
            ..
        }
        | CommunityRequest::ChannelSetTopic {
            community: target,
            channel_id: channel,
            ..
        }
        | CommunityRequest::ChannelSetForumTags {
            community: target,
            channel_id: channel,
            ..
        }
        | CommunityRequest::ChannelOverwriteSet {
            community: target,
            channel_id: channel,
            ..
        }
        | CommunityRequest::ChannelOverwriteDelete {
            community: target,
            channel_id: channel,
            ..
        }
        | CommunityRequest::MekRotate {
            community: target,
            channel,
        } => {
            let (id, community_name) = community(ctx, target)?;
            s.subject = Subject::Channel {
                name: channel_name(ctx, &id, channel),
                community_name,
                id: channel.clone(),
            };
            s.notifies = members(ctx, &id);
            if matches!(request, CommunityRequest::ChannelUpdate { .. }) {
                s.reversibility = Reversibility::Undoable;
            }
            if matches!(request, CommunityRequest::ChannelDelete { .. }) {
                s.escalations = channel_delete_escalations(ctx, &id, channel).await?;
            }
        }
        CommunityRequest::ChannelReorder {
            community: target, ..
        }
        | CommunityRequest::CategoryCreate {
            community: target, ..
        }
        | CommunityRequest::CategoryDelete {
            community: target, ..
        }
        | CommunityRequest::CategoryRename {
            community: target, ..
        }
        | CommunityRequest::CategoryReorder {
            community: target, ..
        } => {
            let (id, name) = community(ctx, target)?;
            if matches!(request, CommunityRequest::CategoryCreate { .. }) {
                s.reversibility = Reversibility::ReversibleByInverse(IntentKind::DeleteCategory);
            }
            s.notifies = members(ctx, &id);
            s.subject = Subject::Community { id, name };
        }
        CommunityRequest::RoleCreate {
            community: target,
            name,
            ..
        } => {
            let (id, _) = community(ctx, target)?;
            s.subject = Subject::Role {
                id: String::new(),
                name: name.clone(),
            };
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::DeleteRole);
            s.notifies = members(ctx, &id);
        }
        CommunityRequest::RoleUpdate {
            community: target,
            role_id,
            ..
        }
        | CommunityRequest::RoleDelete {
            community: target,
            role_id,
        } => {
            let (id, _) = community(ctx, target)?;
            let (name, holders) = role(ctx, &id, *role_id)?;
            if matches!(request, CommunityRequest::RoleDelete { .. }) {
                s.facts.push(Fact::MembersAffected { count: holders });
            } else {
                s.reversibility = Reversibility::Undoable;
            }
            s.subject = Subject::Role {
                id: role_id.to_string(),
                name,
            };
            s.notifies = members(ctx, &id);
        }
        CommunityRequest::RoleSelfAssign {
            community: target,
            role_id,
        }
        | CommunityRequest::RoleSelfUnassign {
            community: target,
            role_id,
        } => {
            let (id, _) = community(ctx, target)?;
            let (name, _) = role(ctx, &id, *role_id)?;
            s.subject = Subject::Role {
                id: role_id.to_string(),
                name,
            };
            s.reversibility = if matches!(request, CommunityRequest::RoleSelfAssign { .. }) {
                Reversibility::ReversibleByInverse(IntentKind::SelfUnassignRole)
            } else {
                Reversibility::ReversibleByInverse(IntentKind::SelfAssignRole)
            };
            s.notifies = members(ctx, &id);
        }
        CommunityRequest::RoleAssign { .. }
        | CommunityRequest::RoleUnassign { .. }
        | CommunityRequest::Kick { .. }
        | CommunityRequest::Ban { .. }
        | CommunityRequest::Unban { .. }
        | CommunityRequest::Timeout { .. }
        | CommunityRequest::TimeoutRemove { .. } => member_selection(ctx, request, s)?,
        CommunityRequest::InviteCreate {
            community: target, ..
        }
        | CommunityRequest::InviteRevoke {
            community: target, ..
        } => {
            let (id, name) = community(ctx, target)?;
            if matches!(request, CommunityRequest::InviteCreate { .. }) {
                s.reversibility =
                    Reversibility::ReversibleByInverse(IntentKind::RevokeCommunityInvite);
            }
            s.subject = Subject::Community { id, name };
        }
        _ => {}
    }
    Ok(())
}

/// Fill `s` for a role grant or a moderation request on one member.
fn member_selection(
    ctx: &DaemonContext,
    request: &CommunityRequest,
    s: &mut Selection,
) -> Result<(), IpcResponse> {
    match request {
        CommunityRequest::RoleAssign {
            community: target,
            member_pseudonym,
            ..
        }
        | CommunityRequest::RoleUnassign {
            community: target,
            member_pseudonym,
            ..
        }
        | CommunityRequest::Kick {
            community: target,
            target_pseudonym: member_pseudonym,
        }
        | CommunityRequest::Ban {
            community: target,
            target_pseudonym: member_pseudonym,
            ..
        }
        | CommunityRequest::Unban {
            community: target,
            target_pseudonym: member_pseudonym,
        }
        | CommunityRequest::Timeout {
            community: target,
            target_pseudonym: member_pseudonym,
            ..
        }
        | CommunityRequest::TimeoutRemove {
            community: target,
            target_pseudonym: member_pseudonym,
        } => {
            let (id, _) = community(ctx, target)?;
            s.subject = Subject::Named {
                name: member_pseudonym.clone(),
            };
            s.reversibility = match request {
                CommunityRequest::RoleAssign { .. } => {
                    Reversibility::ReversibleByInverse(IntentKind::UnassignRole)
                }
                CommunityRequest::RoleUnassign { .. } => {
                    Reversibility::ReversibleByInverse(IntentKind::AssignRole)
                }
                CommunityRequest::Ban { .. } => {
                    Reversibility::ReversibleByInverse(IntentKind::UnbanMember)
                }
                CommunityRequest::Unban { .. } => {
                    Reversibility::ReversibleByInverse(IntentKind::BanMember)
                }
                CommunityRequest::Timeout { .. } => {
                    Reversibility::ReversibleByInverse(IntentKind::RemoveTimeout)
                }
                CommunityRequest::TimeoutRemove { .. } => {
                    Reversibility::ReversibleByInverse(IntentKind::TimeoutMember)
                }
                CommunityRequest::Kick { .. } => {
                    s.facts.push(Fact::RejoinNeedsInvite);
                    Reversibility::NeedsOthers
                }
                _ => Reversibility::Undoable,
            };
            s.notifies = members(ctx, &id);
        }
        _ => {}
    }
    Ok(())
}

/// A community by governance key or name: `(id, name)`.
fn community(ctx: &DaemonContext, target: &str) -> Result<(String, String), IpcResponse> {
    ctx.resolve_community(target)
        .map(|m| (m.governance_key, m.community_name))
}

/// The members a community action reaches (its known members, the
/// validated roster in the daemon).
fn members(ctx: &DaemonContext, community_id: &str) -> Notifies {
    Notifies::Members(u32::try_from(ctx.communities.member_count(community_id)).unwrap_or(u32::MAX))
}

fn channel_name(ctx: &DaemonContext, community_id: &str, channel_id: &str) -> String {
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
    let mut members: Vec<_> = ctx
        .communities
        .known_members(community_id)
        .iter()
        .filter_map(|key| pseudonym(key))
        .collect();
    members.extend(pseudonym(&ctx.my_pseudonym(community_id)));
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

fn pseudonym(hex_key: &str) -> Option<rekindle_types::id::PseudonymKey> {
    hex::decode(hex_key)
        .ok()
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .map(rekindle_types::id::PseudonymKey)
}

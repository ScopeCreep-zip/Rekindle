//! Step 1, Select (plan D3.3, ADR 0019): the facts the host knows about a
//! request that is an intent, assembled from the crates that own them, for
//! the gate in `HostApi::call`. They are computed again at commit, so a
//! review whose facts changed is asked again.
//!
//! Facts the host cannot produce yet join with the port that brings their
//! producer (the channel message count at E3.1, calls and uploads at
//! E4.1/E5.1): none is a placeholder.

use rekindle_idempotency::intent_gate::Selection;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::intent::{Escalation, Fact, IntentKind, Notifies, Reversibility, Subject};

use crate::daemon::dispatch::DaemonContext;

mod friends;

/// The selection for `request`, an intent of `kind`.
///
/// # Errors
/// A fact the review needs cannot be read (the database, or a community
/// whose governance is not loaded): the request is refused rather than
/// reviewed without it.
pub(crate) async fn select(
    ctx: &DaemonContext,
    request: &IpcRequest,
    kind: IntentKind,
) -> Result<Selection, IpcResponse> {
    let mut s = Selection {
        kind,
        subject: Subject::Named {
            name: request.name().to_string(),
        },
        facts: Vec::new(),
        escalations: Vec::new(),
        reversibility: Reversibility::Never,
        notifies: Notifies::Nobody,
    };
    match request {
        IpcRequest::Unlock { identity, .. } | IpcRequest::IdentityDelete { identity } => {
            s.subject = own_identity(ctx, identity)?;
        }
        IpcRequest::Lock | IpcRequest::Logout => {
            s.subject = unlocked_identity(ctx);
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::Login);
            let unsent = unsent(ctx).await?;
            // The daemon holds no call and uploads nothing until E4.1 and
            // E5.1 bring those producers: both are true facts today.
            if unsent > 0 {
                s.escalations.push(Escalation::PendingWork {
                    unsent,
                    in_call: false,
                    uploading: false,
                });
            }
        }
        IpcRequest::IdentityExport { .. } | IpcRequest::IdentityRotate { .. } => {
            s.subject = unlocked_identity(ctx);
            if matches!(request, IpcRequest::IdentityRotate { .. }) {
                s.notifies = Notifies::Members(friend_count(ctx).await?);
            }
        }
        IpcRequest::IdentityWipe => {
            s.subject = Subject::Named {
                name: "every identity on this device".into(),
            };
        }
        IpcRequest::IdentityCreate { display_name, .. } => {
            s.subject = Subject::OwnIdentity {
                name: display_name
                    .clone()
                    .unwrap_or_else(|| "a new identity".into()),
            };
        }
        IpcRequest::FriendAddFromCode { .. }
        | IpcRequest::FriendAddFromInvite { .. }
        | IpcRequest::FriendCancelRequest { .. }
        | IpcRequest::FriendRetryHandshake { .. }
        | IpcRequest::FriendInviteCreate
        | IpcRequest::FriendTokenRotate
        | IpcRequest::FriendInviteCancel { .. }
        | IpcRequest::FriendBlock { .. }
        | IpcRequest::FriendUnblock { .. }
        | IpcRequest::FriendSetNickname { .. }
        | IpcRequest::FriendGroupMove { .. }
        | IpcRequest::FriendGroupCreate { .. }
        | IpcRequest::FriendGroupRename { .. }
        | IpcRequest::FriendGroupDelete { .. }
        | IpcRequest::SessionResetStart { .. }
        | IpcRequest::SessionReinitiate { .. }
        | IpcRequest::SessionResetAccept { .. }
        | IpcRequest::IdentityRotationAccept { .. }
        | IpcRequest::IdentityRotationDecline { .. }
        | IpcRequest::SessionResetDecline { .. }
        | IpcRequest::FriendAccept { .. }
        | IpcRequest::FriendReject { .. }
        | IpcRequest::FriendRemove { .. }
        | IpcRequest::IdentityMarkVerified { .. }
        | IpcRequest::IdentityClearVerification { .. }
        | IpcRequest::DmSend { .. }
        | IpcRequest::OutboxRetry { .. }
        | IpcRequest::OutboxDiscard { .. } => friends::selection(ctx, request, &mut s).await?,
        IpcRequest::CommunityCreate { name, .. } => {
            s.subject = Subject::Community {
                id: String::new(),
                name: name.clone(),
            };
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::LeaveCommunity);
        }
        IpcRequest::CommunityJoin { .. } => {
            s.subject = Subject::Named {
                name: "the community this invite opens".into(),
            };
            s.facts.push(Fact::PresenceVisible);
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::LeaveCommunity);
            s.notifies = Notifies::Subject;
        }
        IpcRequest::CommunityLeave { governance_key } => {
            let (id, name) = community(ctx, governance_key)?;
            s.escalations = leave_escalations(ctx, &id)?;
            s.facts.push(Fact::RejoinNeedsInvite);
            s.reversibility = Reversibility::NeedsOthers;
            s.notifies = members(ctx, &id);
            s.subject = Subject::Community { id, name };
        }
        IpcRequest::CommunityApprove {
            governance_key,
            member_pseudonym,
        }
        | IpcRequest::CommunityReject {
            governance_key,
            member_pseudonym,
            ..
        }
        | IpcRequest::CommunityTransferOwnership {
            governance_key,
            new_owner_pseudonym: member_pseudonym,
        } => {
            let (id, _) = community(ctx, governance_key)?;
            s.subject = Subject::Named {
                name: member_pseudonym.clone(),
            };
            s.notifies = members(ctx, &id);
        }
        IpcRequest::ChannelCreate { .. }
        | IpcRequest::ChannelDelete { .. }
        | IpcRequest::ChannelUpdate { .. }
        | IpcRequest::ChannelSend { .. }
        | IpcRequest::MekRotate { .. }
        | IpcRequest::RoleCreate { .. }
        | IpcRequest::RoleUpdate { .. }
        | IpcRequest::RoleDelete { .. }
        | IpcRequest::RoleAssign { .. }
        | IpcRequest::RoleUnassign { .. }
        | IpcRequest::Kick { .. }
        | IpcRequest::Ban { .. }
        | IpcRequest::Unban { .. }
        | IpcRequest::Timeout { .. }
        | IpcRequest::InviteCreate { .. }
        | IpcRequest::InviteRevoke { .. } => community_selection(ctx, request, &mut s)?,
        IpcRequest::PresenceSet { .. }
        | IpcRequest::PresenceSetMessage { .. }
        | IpcRequest::ProfileSetNickname { .. }
        | IpcRequest::GamePresenceSet { .. }
        | IpcRequest::GamePresenceClear => {
            s.subject = unlocked_identity(ctx);
            s.reversibility = Reversibility::Undoable;
        }
        IpcRequest::SetReadReceiptsEnabled { .. } => {
            s.subject = unlocked_identity(ctx);
            s.reversibility =
                Reversibility::ReversibleByInverse(IntentKind::SetReadReceiptsEnabled);
        }
        IpcRequest::ProfileSetAvatar { .. } => {
            s.subject = unlocked_identity(ctx);
            // The old picture is not kept: another one replaces it.
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::SetAvatar);
        }
        IpcRequest::AuditExport { .. } => s.subject = unlocked_identity(ctx),
        IpcRequest::LinkOpen { url } => {
            // `validate_request` passed it; the host as the browser will
            // ask for it (punycode for an international name).
            let host = rekindle_link_preview::https_url(url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_owned))
                .ok_or_else(|| IpcResponse::error(400, "invalid link"))?;
            s.subject = Subject::Link { host };
            s.facts.push(Fact::IpAddressLeaves);
        }
        IpcRequest::VoiceJoin { channel, .. } => {
            s.subject = Subject::Named {
                name: channel.clone(),
            };
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::LeaveVoice);
        }
        IpcRequest::AgentRegister { name, .. } => {
            s.subject = Subject::Named { name: name.clone() };
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::RevokeAgent);
        }
        IpcRequest::AgentRevoke { name } => {
            s.subject = Subject::Named { name: name.clone() };
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::RegisterAgent);
        }
        // The request itself is the subject (shutting the host down,
        // importing an archive, reloading policy, leaving voice), and the
        // rest are not intents (`COMMAND_TABLE`): the gate never selects
        // them.
        IpcRequest::Shutdown
        | IpcRequest::IdentityImport { .. }
        | IpcRequest::PolicyReload
        | IpcRequest::VoiceLeave
        | IpcRequest::Status
        | IpcRequest::IdentityList
        | IpcRequest::IdentityShow
        | IpcRequest::PrekeyStatus
        | IpcRequest::FriendCode
        | IpcRequest::PresenceGet
        | IpcRequest::AvatarGet { .. }
        | IpcRequest::AuditVerify
        | IpcRequest::FriendList
        | IpcRequest::FriendRequests
        | IpcRequest::FriendInviteList
        | IpcRequest::FriendBlockedList
        | IpcRequest::IdentitySafetyNumber { .. }
        | IpcRequest::DmHistory { .. }
        | IpcRequest::DmMarkRead { .. }
        | IpcRequest::CommunityList
        | IpcRequest::CommunityInfo { .. }
        | IpcRequest::CommunityPendingMembers { .. }
        | IpcRequest::ChannelList { .. }
        | IpcRequest::ChannelHistory { .. }
        | IpcRequest::DmTyping { .. }
        | IpcRequest::DmInbox { .. }
        | IpcRequest::IntentCommit { .. }
        | IpcRequest::IntentCancel { .. }
        | IpcRequest::Undo { .. }
        | IpcRequest::Subscribe { .. }
        | IpcRequest::Unsubscribe { .. }
        | IpcRequest::MekList { .. }
        | IpcRequest::MekRequest { .. }
        | IpcRequest::RoleList { .. }
        | IpcRequest::BanList { .. }
        | IpcRequest::InviteList { .. }
        | IpcRequest::NetworkStatus
        | IpcRequest::NetworkPeers => {}
    }
    Ok(s)
}

/// The selection of a community, channel, role or moderation request.
fn community_selection(
    ctx: &DaemonContext,
    request: &IpcRequest,
    s: &mut Selection,
) -> Result<(), IpcResponse> {
    match request {
        IpcRequest::ChannelCreate {
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
        IpcRequest::ChannelDelete {
            community: target,
            channel_id: channel,
        }
        | IpcRequest::ChannelUpdate {
            community: target,
            channel_id: channel,
            ..
        }
        | IpcRequest::ChannelSend {
            community: target,
            channel,
            ..
        }
        | IpcRequest::MekRotate {
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
            if matches!(request, IpcRequest::ChannelUpdate { .. }) {
                s.reversibility = Reversibility::Undoable;
            }
        }
        IpcRequest::RoleCreate {
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
        IpcRequest::RoleUpdate {
            community: target,
            role_id,
            ..
        }
        | IpcRequest::RoleDelete {
            community: target,
            role_id,
        } => {
            let (id, _) = community(ctx, target)?;
            let (name, holders) = role(ctx, &id, *role_id)?;
            if matches!(request, IpcRequest::RoleDelete { .. }) {
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
        IpcRequest::RoleAssign {
            community: target,
            member_pseudonym,
            ..
        }
        | IpcRequest::RoleUnassign {
            community: target,
            member_pseudonym,
            ..
        }
        | IpcRequest::Kick {
            community: target,
            target_pseudonym: member_pseudonym,
        }
        | IpcRequest::Ban {
            community: target,
            target_pseudonym: member_pseudonym,
            ..
        }
        | IpcRequest::Unban {
            community: target,
            target_pseudonym: member_pseudonym,
        }
        | IpcRequest::Timeout {
            community: target,
            target_pseudonym: member_pseudonym,
            ..
        } => {
            let (id, _) = community(ctx, target)?;
            s.subject = Subject::Named {
                name: member_pseudonym.clone(),
            };
            s.reversibility = match request {
                IpcRequest::RoleAssign { .. } => {
                    Reversibility::ReversibleByInverse(IntentKind::UnassignRole)
                }
                IpcRequest::RoleUnassign { .. } => {
                    Reversibility::ReversibleByInverse(IntentKind::AssignRole)
                }
                IpcRequest::Ban { .. } => {
                    Reversibility::ReversibleByInverse(IntentKind::UnbanMember)
                }
                IpcRequest::Unban { .. } => {
                    Reversibility::ReversibleByInverse(IntentKind::BanMember)
                }
                IpcRequest::Kick { .. } => {
                    s.facts.push(Fact::RejoinNeedsInvite);
                    Reversibility::NeedsOthers
                }
                _ => Reversibility::Undoable,
            };
            s.notifies = members(ctx, &id);
        }
        IpcRequest::InviteCreate {
            community: target, ..
        }
        | IpcRequest::InviteRevoke {
            community: target, ..
        } => {
            let (id, name) = community(ctx, target)?;
            if matches!(request, IpcRequest::InviteCreate { .. }) {
                s.reversibility =
                    Reversibility::ReversibleByInverse(IntentKind::RevokeCommunityInvite);
            }
            s.subject = Subject::Community { id, name };
        }
        _ => {}
    }
    Ok(())
}

fn unavailable(what: &str, e: impl std::fmt::Display) -> IpcResponse {
    IpcResponse::error_with_remediation(
        503,
        format!("{what} cannot be read for review: {e}"),
        "retry once the identity's state has loaded",
    )
}

/// The request that takes `request` back, computed from live state when
/// the action is selected (plan E1.1): the status, message, name or game it
/// replaces. `None` for an action that offers no undo.
pub(crate) fn inverse(
    ctx: &DaemonContext,
    request: &IpcRequest,
    reversibility: Reversibility,
) -> Option<IpcRequest> {
    if reversibility != Reversibility::Undoable {
        return None;
    }
    let presence = ctx.own_presence.view();
    match request {
        IpcRequest::PresenceSet { .. } => Some(IpcRequest::PresenceSet {
            status: presence.status,
        }),
        IpcRequest::PresenceSetMessage { .. } => Some(IpcRequest::PresenceSetMessage {
            message: presence.status_message,
        }),
        IpcRequest::ProfileSetNickname { .. } => Some(IpcRequest::ProfileSetNickname {
            display_name: ctx.identity_display_name(),
        }),
        IpcRequest::FriendSetNickname { public_key, .. } => {
            let current = ctx
                .session
                .read()
                .as_ref()
                .and_then(|s| s.friends.get(public_key).map(|f| f.nickname.clone()))?;
            Some(IpcRequest::FriendSetNickname {
                public_key: public_key.clone(),
                nickname: current,
            })
        }
        IpcRequest::GamePresenceSet { .. } | IpcRequest::GamePresenceClear => {
            Some(presence.game.map_or(IpcRequest::GamePresenceClear, |game| {
                IpcRequest::GamePresenceSet {
                    game_name: game.game_name,
                    game_id: (game.game_id != 0).then_some(game.game_id),
                    elapsed_seconds: game.elapsed_seconds,
                    server_address: game.server_address,
                }
            }))
        }
        _ => None,
    }
}

/// The unlocked identity, by its own name.
fn unlocked_identity(ctx: &DaemonContext) -> Subject {
    Subject::OwnIdentity {
        name: ctx.identity_display_name(),
    }
}

/// An identity of this root, by the name its registry entry holds.
fn own_identity(ctx: &DaemonContext, public_key: &str) -> Result<Subject, IpcResponse> {
    let entries = super::identity::list(&ctx.root).map_err(|e| unavailable("the identities", e))?;
    let name = entries
        .into_iter()
        .find(|entry| entry.public_key == public_key)
        .map(|entry| entry.display_name)
        .ok_or_else(|| IpcResponse::error(404, "no such identity"))?;
    Ok(Subject::OwnIdentity { name })
}

/// What a lock or logout leaves undelivered (`repo::intent_facts::unsent`).
async fn unsent(ctx: &DaemonContext) -> Result<u32, IpcResponse> {
    let (Ok(owner), Ok(db)) = (ctx.require_owner(), ctx.require_db()) else {
        return Ok(0);
    };
    db.call(move |conn| rekindle_db::repo::intent_facts::unsent(conn, &owner))
        .await
        .map_err(|e| unavailable("the unsent work", e))
}

async fn friend_count(ctx: &DaemonContext) -> Result<u32, IpcResponse> {
    let (Ok(owner), Ok(db)) = (ctx.require_owner(), ctx.require_db()) else {
        return Ok(0);
    };
    let friends = db
        .call(move |conn| rekindle_db::repo::friends::load_all(conn, &owner))
        .await
        .map_err(|e| unavailable("the friends", e))?;
    Ok(u32::try_from(friends.len()).unwrap_or(u32::MAX))
}

/// A community by governance key or name: `(id, name)`.
fn community(ctx: &DaemonContext, target: &str) -> Result<(String, String), IpcResponse> {
    ctx.resolve_community(target)
        .map(|m| (m.governance_key, m.community_name))
}

/// The members a community action reaches (its validated roster).
fn members(ctx: &DaemonContext, community_id: &str) -> Notifies {
    Notifies::Members(
        u32::try_from(ctx.community_runtime.member_count(community_id)).unwrap_or(u32::MAX),
    )
}

fn channel_name(ctx: &DaemonContext, community_id: &str, channel_id: &str) -> String {
    let parsed = hex::decode(channel_id)
        .ok()
        .and_then(|bytes| <[u8; 16]>::try_from(bytes).ok())
        .map(rekindle_types::id::ChannelId);
    parsed
        .and_then(|id| {
            ctx.community_runtime
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
        .community_runtime
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
        .community_runtime
        .governance_state(community_id)
        .ok_or_else(|| unavailable("the community's governance", "not loaded"))?;
    let me = pseudonym(&ctx.my_pseudonym(community_id))
        .ok_or_else(|| unavailable("our pseudonym", "not in the community"))?;
    let members: Vec<_> = ctx
        .community_runtime
        .members(community_id)
        .keys()
        .filter_map(|key| pseudonym(key))
        .collect();
    Ok(rekindle_governance::intent_facts::leave_escalations(
        &state,
        &me,
        &members,
        ctx.clock.now_secs(),
    ))
}

fn pseudonym(hex_key: &str) -> Option<rekindle_types::id::PseudonymKey> {
    hex::decode(hex_key)
        .ok()
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .map(rekindle_types::id::PseudonymKey)
}

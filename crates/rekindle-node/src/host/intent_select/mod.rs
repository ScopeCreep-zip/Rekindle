//! Step 1, Select (plan D3.3, ADR 0019): the facts the host knows about a
//! request that is an intent, assembled from the crates that own them, for
//! the gate in `HostApi::call`. They are computed again at commit, so a
//! review whose facts changed is asked again.
//!
//! Facts the host cannot produce yet join with the port that brings their
//! producer (the channel message count at E3.1, calls and uploads at
//! E4.1/E5.1): none is a placeholder.

use rekindle_idempotency::intent_gate::Selection;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, IpcResponse};
use rekindle_types::intent::{Escalation, Fact, IntentKind, Notifies, Reversibility, Subject};

use crate::daemon::dispatch::DaemonContext;

mod community;
mod community_messages;
mod events;
mod friends;
mod game_servers;
mod group_dm;
mod rich_messaging;

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
        IpcRequest::GroupDmCreate { .. }
        | IpcRequest::GroupDmAccept { .. }
        | IpcRequest::GroupDmDecline { .. }
        | IpcRequest::GroupDmLeave { .. }
        | IpcRequest::GroupDmSend { .. } => {
            group_dm::selection(ctx, request, &mut s).await?;
        }
        IpcRequest::Community(
            request @ (CommunityRequest::CommunityCreate { .. }
            | CommunityRequest::CommunityJoin { .. }
            | CommunityRequest::CommunityLeave { .. }
            | CommunityRequest::CommunityApprove { .. }
            | CommunityRequest::CommunityReject { .. }
            | CommunityRequest::CommunityTransferOwnership { .. }
            | CommunityRequest::ChannelCreate { .. }
            | CommunityRequest::ChannelDelete { .. }
            | CommunityRequest::ChannelUpdate { .. }
            | CommunityRequest::ChannelRename { .. }
            | CommunityRequest::ChannelMove { .. }
            | CommunityRequest::ChannelReorder { .. }
            | CommunityRequest::ChannelSetTopic { .. }
            | CommunityRequest::ChannelSetSlowmode { .. }
            | CommunityRequest::ChannelSetForumTags { .. }
            | CommunityRequest::ChannelOverwriteSet { .. }
            | CommunityRequest::ChannelOverwriteDelete { .. }
            | CommunityRequest::CategoryCreate { .. }
            | CommunityRequest::CategoryDelete { .. }
            | CommunityRequest::CategoryRename { .. }
            | CommunityRequest::CategoryReorder { .. }
            | CommunityRequest::ChannelSend { .. }
            | CommunityRequest::ChannelMessageEdit { .. }
            | CommunityRequest::ChannelMessageDelete { .. }
            | CommunityRequest::ChannelMessageForward { .. }
            | CommunityRequest::ChannelMessageAdminDelete { .. }
            | CommunityRequest::ChannelMessageBulkDelete { .. }
            | CommunityRequest::ChannelMessageSend { .. }
            | CommunityRequest::MekRotate { .. }
            | CommunityRequest::RoleCreate { .. }
            | CommunityRequest::RoleUpdate { .. }
            | CommunityRequest::RoleDelete { .. }
            | CommunityRequest::RoleAssign { .. }
            | CommunityRequest::RoleUnassign { .. }
            | CommunityRequest::RoleSelfAssign { .. }
            | CommunityRequest::RoleSelfUnassign { .. }
            | CommunityRequest::Kick { .. }
            | CommunityRequest::MemberBlock { .. }
            | CommunityRequest::MemberUnblock { .. }
            | CommunityRequest::Ban { .. }
            | CommunityRequest::Unban { .. }
            | CommunityRequest::Timeout { .. }
            | CommunityRequest::TimeoutRemove { .. }
            | CommunityRequest::InviteCreate { .. }
            | CommunityRequest::InviteRevoke { .. }
            | CommunityRequest::NotificationDefaultSet { .. }
            | CommunityRequest::AutomodRuleSet { .. }
            | CommunityRequest::AutomodRuleDelete { .. }
            | CommunityRequest::MessagePin { .. }
            | CommunityRequest::MessageUnpin { .. }
            | CommunityRequest::AttachmentPin { .. }
            | CommunityRequest::EmojiUpload { .. }
            | CommunityRequest::StickerUpload { .. }
            | CommunityRequest::ExpressionDelete { .. }),
        ) => community::selection(ctx, request, &mut s).await?,
        IpcRequest::Community(
            request @ (CommunityRequest::ReactionAdd { .. }
            | CommunityRequest::ReactionRemove { .. }
            | CommunityRequest::ThreadCreate { .. }
            | CommunityRequest::ThreadArchive { .. }
            | CommunityRequest::ThreadUnarchive { .. }
            | CommunityRequest::ThreadSend { .. }
            | CommunityRequest::PollCreate { .. }
            | CommunityRequest::PollVote { .. }
            | CommunityRequest::PollClose { .. }),
        ) => rich_messaging::selection(ctx, request, &mut s)?,
        IpcRequest::Community(
            request @ (CommunityRequest::CommunityUpdateProfile { .. }
            | CommunityRequest::CommunitySetAvatar { .. }
            | CommunityRequest::CommunitySetBanner { .. }
            | CommunityRequest::CommunityPolicySet { .. }
            | CommunityRequest::OnboardingConfigSet { .. }
            | CommunityRequest::WelcomeScreenSet { .. }
            | CommunityRequest::OnboardingSubmit { .. }
            | CommunityRequest::OnboardingComplete { .. }),
        ) => community::settings_selection(ctx, request, &mut s)?,
        IpcRequest::Community(
            request @ (CommunityRequest::GameServerAdd { .. }
            | CommunityRequest::GameServerRemove { .. }),
        ) => {
            game_servers::selection(ctx, request, &mut s).await?;
        }
        IpcRequest::Community(
            request @ (CommunityRequest::EventCreate { .. }
            | CommunityRequest::EventEdit { .. }
            | CommunityRequest::EventCancel { .. }
            | CommunityRequest::EventDelete { .. }
            | CommunityRequest::EventRsvp { .. }),
        ) => {
            events::selection(ctx, request, &mut s).await?;
        }
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
        | IpcRequest::GroupDmList
        | IpcRequest::GroupDmMessages { .. }
        | IpcRequest::Community(
            CommunityRequest::CommunityList
            | CommunityRequest::CommunityViews
            | CommunityRequest::CommunityMembers { .. }
            | CommunityRequest::CommunityInfo { .. }
            | CommunityRequest::CommunityUpdateInfo { .. }
            | CommunityRequest::CommunityPendingMembers { .. }
            | CommunityRequest::ChannelList { .. }
            | CommunityRequest::ChannelTyping { .. }
            | CommunityRequest::ChannelSetActive { .. }
            | CommunityRequest::ChannelMessages { .. }
            | CommunityRequest::ChannelMessagesBefore { .. }
            | CommunityRequest::MekList { .. }
            | CommunityRequest::MekRequest { .. }
            | CommunityRequest::RoleList { .. }
            | CommunityRequest::GameServerList { .. }
            | CommunityRequest::ChannelMarkRead { .. }
            | CommunityRequest::UnreadCounts { .. }
            | CommunityRequest::OnboardingConfigGet { .. }
            | CommunityRequest::WelcomeScreenGet { .. }
            | CommunityRequest::AuditLog { .. }
            | CommunityRequest::NotificationDefaultGet { .. }
            | CommunityRequest::CommunityAvatarGet { .. }
            | CommunityRequest::CommunityPolicyGet { .. }
            | CommunityRequest::EventList { .. }
            | CommunityRequest::EventAttendees { .. }
            | CommunityRequest::BanList { .. }
            | CommunityRequest::MemberBlockList { .. }
            | CommunityRequest::AutomodRules { .. }
            | CommunityRequest::ModReportList { .. }
            | CommunityRequest::ChannelPins { .. }
            | CommunityRequest::MessageSearch { .. }
            | CommunityRequest::ThreadList { .. }
            | CommunityRequest::ThreadListActive { .. }
            | CommunityRequest::ThreadMessages { .. }
            | CommunityRequest::PollResults { .. }
            | CommunityRequest::Expressions { .. }
            | CommunityRequest::InviteList { .. },
        )
        | IpcRequest::DmTyping { .. }
        | IpcRequest::GroupDmTyping { .. }
        | IpcRequest::DmInbox { .. }
        | IpcRequest::IntentCommit { .. }
        | IpcRequest::IntentCancel { .. }
        | IpcRequest::Undo { .. }
        | IpcRequest::Subscribe { .. }
        | IpcRequest::Unsubscribe { .. }
        | IpcRequest::NetworkStatus
        | IpcRequest::NetworkPeers => {}
    }
    Ok(s)
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

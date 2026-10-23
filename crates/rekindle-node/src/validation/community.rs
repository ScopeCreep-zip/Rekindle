//! A community's requests (plan E3.1): its governance key, names, member
//! pseudonyms, roles, moderation, invites and channel keys; channels are
//! `channels`'.

use rekindle_governance::validate::bounds;
use rekindle_ipc::protocol::{
    ActivityRequest, AuditRequest, ExpressionRequest, PinRequest, SettingsRequest, UnreadRequest,
};
use rekindle_ipc::protocol::{
    ChannelMessageRequest, ChannelRequest, LifecycleRequest, MemberRequest, RichRequest,
};
use rekindle_ipc::protocol::{CommunityRequest, IpcResponse};
use rekindle_types::governance::ExclusionGroupEdit;
use rekindle_types::invite::InviteLink;
use rekindle_types::key_format::{self, MAX_NOTE_LEN};
use rekindle_types::member::MAX_TIMEOUT_SECONDS;

/// The longest exclusion-group slug in bytes (the Host normalizes it to at
/// most 32 characters of ASCII).
const MAX_EXCLUSION_GROUP_LEN: usize = 32;

/// The longest community policy text, in characters (architecture §10.7).
const MAX_POLICY_TEXT_CHARS: usize = 4096;

/// The longest channel or message id a read marker names.
const MAX_UNREAD_ID_LEN: usize = 128;

use crate::host::adapters::governance::community_pictures::{
    is_picture_hash, MAX_UPLOAD_BYTES as MAX_PICTURE_UPLOAD_BYTES,
};

use super::community_fields::{audit_fields, automod_fields, game_server_fields};
use super::{community, key_scope, name, opt, pseudonym, reject, text, Checked};

/// A community request's fields.
/// A read-marker or unread-count request (plan E3.1 G1).
fn unread_fields(request: &UnreadRequest) -> Checked {
    match request {
        UnreadRequest::ChannelMarkRead {
            community: key,
            channel,
            up_to,
        } => {
            community(key)?;
            text(channel, MAX_UNREAD_ID_LEN, "channel")?;
            text(up_to, MAX_UNREAD_ID_LEN, "message id")
        }
        UnreadRequest::UnreadCounts { community: key } => community(key),
    }
}

/// A community block request (plan E3.1 F3, D26).
fn block_fields(request: &MemberRequest) -> Checked {
    match request {
        MemberRequest::MemberBlock {
            community: c,
            target_pseudonym,
        }
        | MemberRequest::MemberUnblock {
            community: c,
            target_pseudonym,
        } => {
            community(c)?;
            pseudonym(target_pseudonym, "target pseudonym")
        }
        MemberRequest::MemberBlockList { community: c } => community(c),
        _ => Ok(()),
    }
}

pub(super) fn check(request: &CommunityRequest) -> Checked {
    match request {
        CommunityRequest::Lifecycle(request) => check_lifecycle(request),
        CommunityRequest::Settings(request) => check_settings(request),
        CommunityRequest::Channels(request) => check_channels(request),
        CommunityRequest::Messages(request) => check_messages(request),
        CommunityRequest::Unread(request) => check_unread(request),
        CommunityRequest::Audit(request) => check_audit(request),
        CommunityRequest::Activities(request) => check_activities(request),
        CommunityRequest::Members(request) => check_members(request),
        CommunityRequest::Pins(request) => check_pins(request),
        CommunityRequest::Rich(request) => check_rich(request),
        CommunityRequest::Expressions(request) => check_expressions(request),
    }
}

/// `check` for the community lifecycle and invite requests.
pub(super) fn check_lifecycle(request: &LifecycleRequest) -> Checked {
    match request {
LifecycleRequest::CommunityList | LifecycleRequest::CommunityViews => Ok(()),
LifecycleRequest::CommunityCreate {
            name: community_name,
            description,
            approval_required: _,
        } => {
            name(community_name, "community")?;
            bounds::description(description)
                .map(drop)
                .map_err(|e| reject("description", &e))
        }
LifecycleRequest::CommunityJoin { invite } => InviteLink::parse(invite).map(drop).map_err(|e| {
            IpcResponse::error(
                400,
                format!(
                    "join requires a full invite link \
                     (rekindle://invite/{{governance_key}}/{{secrets_record_key}}/{{invite_code}}): {e}"
                ),
            )
        }),
LifecycleRequest::CommunityLeave { governance_key } | LifecycleRequest::CommunityInfo { governance_key } | LifecycleRequest::CommunityMembers { governance_key } | LifecycleRequest::CommunityPendingMembers { governance_key } => community(governance_key),
LifecycleRequest::CommunityUpdateInfo {
            governance_key,
            name: community_name,
            description,
            icon_hash,
            banner_hash,
        } => {
            community(governance_key)?;
            opt(community_name.as_deref(), |n| name(n, "community"))?;
            opt(description.as_deref(), |d| {
                bounds::description(d)
                    .map(drop)
                    .map_err(|e| reject("description", &e))
            })?;
            opt(icon_hash.as_deref(), |h| blob_hash(h, "icon hash"))?;
            opt(banner_hash.as_deref(), |h| blob_hash(h, "banner hash"))
        }
LifecycleRequest::CommunityApprove {
            governance_key,
            member_pseudonym,
        } => {
            community(governance_key)?;
            pseudonym(member_pseudonym, "member pseudonym")
        }
LifecycleRequest::CommunityReject {
            governance_key,
            member_pseudonym,
            reason,
        } => {
            community(governance_key)?;
            pseudonym(member_pseudonym, "member pseudonym")?;
            text(reason, MAX_NOTE_LEN, "reason")
        }
LifecycleRequest::CommunityTransferOwnership {
            governance_key,
            new_owner_pseudonym,
        } => {
            community(governance_key)?;
            pseudonym(new_owner_pseudonym, "new owner pseudonym")
        }
LifecycleRequest::InviteList { community: c }
        // Role ids, invite use counts and expiries are governance rules,
        // checked where the op is validated.
| LifecycleRequest::InviteCreate {
            community: c,
            max_uses: _,
            expires_seconds: _,
        } => community(c),
LifecycleRequest::InviteRevoke {
            community: c,
            invite_code,
        } => {
            community(c)?;
            key_format::hex16_id(invite_code).map(drop).map_err(|e| {
                IpcResponse::error(
                    400,
                    format!(
                        "invalid invite id: {e} — revoke takes the 16-byte invite id from \
                         `invite list`, not the raw code, which is never published"
                    ),
                )
            })
        }
}
}

/// `check` for the profile, policy and onboarding requests.
pub(super) fn check_settings(request: &SettingsRequest) -> Checked {
    match request {
        SettingsRequest::CommunityPolicyGet { community: key } => community(key),
        SettingsRequest::CommunityUpdateProfile {
            community: key,
            bio,
            pronouns,
            badges,
            avatar_ref,
            banner_ref,
            theme_color: _,
        } => {
            community(key)?;
            rekindle_types::presence::limits::validate_profile(
                bio.as_deref(),
                pronouns.as_deref(),
                badges,
                avatar_ref.as_deref(),
                banner_ref.as_deref(),
            )
            .map_err(|e| IpcResponse::error(400, e))
        }
        SettingsRequest::CommunitySetAvatar {
            community: key,
            image,
            kind: _,
        }
        | SettingsRequest::CommunitySetBanner {
            community: key,
            image,
            kind: _,
        } => {
            community(key)?;
            if image.is_empty() || image.len() > MAX_PICTURE_UPLOAD_BYTES {
                return Err(IpcResponse::error(
                    400,
                    format!("a picture is 1 to {MAX_PICTURE_UPLOAD_BYTES} bytes"),
                ));
            }
            Ok(())
        }
        SettingsRequest::CommunityAvatarGet {
            community: key,
            hash,
        } => {
            community(key)?;
            if is_picture_hash(hash) {
                Ok(())
            } else {
                Err(IpcResponse::error(
                    400,
                    "a picture hash is 64 lowercase hex digits",
                ))
            }
        }
        SettingsRequest::CommunityPolicySet {
            community: key,
            policy_text,
            max_joins_per_interval,
            join_interval_seconds,
        } => {
            community(key)?;
            if *max_joins_per_interval == 0 || *join_interval_seconds == 0 {
                return Err(IpcResponse::error(
                    400,
                    "the raid thresholds must be above 0",
                ));
            }
            opt(policy_text.as_deref(), |t| {
                if t.chars().count() > MAX_POLICY_TEXT_CHARS {
                    Err(IpcResponse::error(
                        400,
                        format!("policy text exceeds {MAX_POLICY_TEXT_CHARS} characters"),
                    ))
                } else {
                    Ok(())
                }
            })
        }
        SettingsRequest::OnboardingConfigGet { .. }
        | SettingsRequest::OnboardingConfigSet { .. }
        | SettingsRequest::WelcomeScreenGet { .. }
        | SettingsRequest::WelcomeScreenSet { .. }
        | SettingsRequest::OnboardingSubmit { .. }
        | SettingsRequest::OnboardingComplete { .. } => super::community_onboarding::check(request),
    }
}

/// `check` for the channel and category requests.
pub(super) fn check_channels(request: &ChannelRequest) -> Checked {
    match request {
        ChannelRequest::ChannelList { community: c } => community(c),
        ChannelRequest::ChannelCreate { .. }
        | ChannelRequest::ChannelDelete { .. }
        | ChannelRequest::ChannelUpdate { .. }
        | ChannelRequest::ChannelRename { .. }
        | ChannelRequest::ChannelMove { .. }
        | ChannelRequest::ChannelReorder { .. }
        | ChannelRequest::ChannelSetTopic { .. }
        | ChannelRequest::ChannelSetSlowmode { .. }
        | ChannelRequest::ChannelSetForumTags { .. }
        | ChannelRequest::ChannelOverwriteSet { .. }
        | ChannelRequest::ChannelOverwriteDelete { .. }
        | ChannelRequest::CategoryCreate { .. }
        | ChannelRequest::CategoryDelete { .. }
        | ChannelRequest::CategoryRename { .. }
        | ChannelRequest::CategoryReorder { .. } => super::channels::check_channels(request),
    }
}

/// `check` for the channel message and channel key requests.
pub(super) fn check_messages(request: &ChannelMessageRequest) -> Checked {
    match request {
        ChannelMessageRequest::MekList { community: c } => community(c),
        ChannelMessageRequest::ChannelSend { .. }
        | ChannelMessageRequest::ChannelMessages { .. }
        | ChannelMessageRequest::ChannelMessagesBefore { .. }
        | ChannelMessageRequest::ChannelMessageEdit { .. }
        | ChannelMessageRequest::ChannelMessageDelete { .. }
        | ChannelMessageRequest::ChannelMessageSend { .. }
        | ChannelMessageRequest::ChannelMessageForward { .. }
        | ChannelMessageRequest::ChannelMessageAdminDelete { .. }
        | ChannelMessageRequest::ChannelMessageBulkDelete { .. }
        | ChannelMessageRequest::ChannelTyping { .. }
        | ChannelMessageRequest::ChannelSetActive { .. } => {
            super::channels::check_messages(request)
        }
        ChannelMessageRequest::MekRotate {
            community: c,
            channel: ch,
        }
        | ChannelMessageRequest::MekRequest {
            community: c,
            channel: ch,
            generation: _,
        } => {
            community(c)?;
            key_scope(ch)
        }
    }
}

/// `check` for the read marker requests.
pub(super) fn check_unread(request: &UnreadRequest) -> Checked {
    match request {
        request @ (UnreadRequest::ChannelMarkRead { .. } | UnreadRequest::UnreadCounts { .. }) => {
            unread_fields(request)
        }
    }
}

/// `check` for the audit and notification default requests.
pub(super) fn check_audit(request: &AuditRequest) -> Checked {
    match request {
        AuditRequest::NotificationDefaultGet { community: key } => community(key),
        request @ (AuditRequest::AuditLog { .. } | AuditRequest::NotificationDefaultSet { .. }) => {
            audit_fields(request)
        }
    }
}

/// `check` for the event and game server requests.
pub(super) fn check_activities(request: &ActivityRequest) -> Checked {
    match request {
        ActivityRequest::GameServerList { community: key } => community(key),
        ActivityRequest::EventList { .. }
        | ActivityRequest::EventAttendees { .. }
        | ActivityRequest::EventCreate { .. }
        | ActivityRequest::EventEdit { .. }
        | ActivityRequest::EventCancel { .. }
        | ActivityRequest::EventDelete { .. }
        | ActivityRequest::EventRsvp { .. } => super::community_events::check(request),
        request @ (ActivityRequest::GameServerAdd { .. }
        | ActivityRequest::GameServerRemove { .. }) => game_server_fields(request),
    }
}

/// `check` for the role and moderation requests.
pub(super) fn check_members(request: &MemberRequest) -> Checked {
    match request {
        MemberRequest::AutomodRules { community: key } => community(key),
        request @ (MemberRequest::MemberBlock { .. }
        | MemberRequest::MemberUnblock { .. }
        | MemberRequest::MemberBlockList { .. }) => block_fields(request),
        request @ (MemberRequest::AutomodRuleSet { .. }
        | MemberRequest::AutomodRuleDelete { .. }) => automod_fields(request),
        MemberRequest::RoleList { community: c }
        | MemberRequest::BanList { community: c }
        | MemberRequest::ModReportList { community: c }
        | MemberRequest::RoleDelete {
            community: c,
            role_id: _,
        }
        | MemberRequest::RoleSelfAssign {
            community: c,
            role_id: _,
        }
        | MemberRequest::RoleSelfUnassign {
            community: c,
            role_id: _,
        } => community(c),
        MemberRequest::RoleCreate {
            community: c,
            name: role_name,
            exclusion_group,
            ..
        } => {
            community(c)?;
            name(role_name, "role")?;
            opt(exclusion_group.as_deref(), |g| {
                text(g, MAX_EXCLUSION_GROUP_LEN, "exclusion group")
            })
        }
        MemberRequest::RoleUpdate {
            community: c,
            role_id: _,
            name: new_name,
            exclusion_group,
            ..
        } => {
            community(c)?;
            opt(new_name.as_deref(), |n| name(n, "role"))?;
            match exclusion_group {
                ExclusionGroupEdit::Set(g) => text(g, MAX_EXCLUSION_GROUP_LEN, "exclusion group"),
                ExclusionGroupEdit::Unchanged | ExclusionGroupEdit::Clear => Ok(()),
            }
        }
        MemberRequest::RoleAssign {
            community: c,
            member_pseudonym,
            role_id: _,
        }
        | MemberRequest::RoleUnassign {
            community: c,
            member_pseudonym,
            role_id: _,
        } => {
            community(c)?;
            pseudonym(member_pseudonym, "member pseudonym")
        }
        MemberRequest::Kick {
            community: c,
            target_pseudonym,
        }
        | MemberRequest::Unban {
            community: c,
            target_pseudonym,
        }
        | MemberRequest::TimeoutRemove {
            community: c,
            target_pseudonym,
        } => {
            community(c)?;
            pseudonym(target_pseudonym, "target pseudonym")
        }
        MemberRequest::Ban {
            community: c,
            target_pseudonym,
            reason,
        } => {
            community(c)?;
            pseudonym(target_pseudonym, "target pseudonym")?;
            opt(reason.as_deref(), |r| text(r, MAX_NOTE_LEN, "reason"))
        }
        MemberRequest::Timeout {
            community: c,
            target_pseudonym,
            duration_seconds,
            reason,
        } => {
            community(c)?;
            pseudonym(target_pseudonym, "target pseudonym")?;
            if !bounds::timeout_ok(*duration_seconds) {
                return Err(IpcResponse::error(
                    400,
                    format!(
                        "timeout must be 1-{MAX_TIMEOUT_SECONDS} seconds, got {duration_seconds}"
                    ),
                ));
            }
            opt(reason.as_deref(), |r| text(r, MAX_NOTE_LEN, "reason"))
        }
    }
}

/// `check` for the pin and search requests.
pub(super) fn check_pins(request: &PinRequest) -> Checked {
    match request {
        request @ (PinRequest::ChannelPins { .. }
        | PinRequest::MessagePin { .. }
        | PinRequest::MessageUnpin { .. }
        | PinRequest::AttachmentPin { .. }
        | PinRequest::AttachmentUpload { .. }
        | PinRequest::AttachmentDownload { .. }
        | PinRequest::AttachmentInfo { .. }
        | PinRequest::VoiceMessageSend { .. }
        | PinRequest::VoiceMessageAudio { .. }
        | PinRequest::MessageSearch { .. }) => super::pins::check(request),
    }
}

/// `check` for the reaction, thread and poll requests.
pub(super) fn check_rich(request: &RichRequest) -> Checked {
    match request {
        RichRequest::ReactionAdd { .. }
        | RichRequest::ReactionRemove { .. }
        | RichRequest::ThreadCreate { .. }
        | RichRequest::ThreadArchive { .. }
        | RichRequest::ThreadUnarchive { .. }
        | RichRequest::ThreadList { .. }
        | RichRequest::ThreadListActive { .. }
        | RichRequest::ThreadMessages { .. }
        | RichRequest::ThreadSend { .. }
        | RichRequest::PollCreate { .. }
        | RichRequest::PollVote { .. }
        | RichRequest::PollClose { .. }
        | RichRequest::PollResults { .. } => super::rich_messaging::check(request),
    }
}

/// `check` for the expression requests.
pub(super) fn check_expressions(request: &ExpressionRequest) -> Checked {
    match request {
        request @ (ExpressionRequest::EmojiUpload { .. }
        | ExpressionRequest::StickerUpload { .. }
        | ExpressionRequest::ExpressionDelete { .. }
        | ExpressionRequest::Expressions { .. }) => super::expressions::check(request),
    }
}

/// A BLAKE3 content hash: 64 lowercase hex.
fn blob_hash(hash: &str, what: &str) -> Checked {
    if hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Ok(())
    } else {
        Err(IpcResponse::error(
            400,
            format!("invalid {what}: 64 lowercase hex"),
        ))
    }
}

//! A community's requests (plan E3.1): its governance key, names, member
//! pseudonyms, roles, moderation, invites and channel keys; channels are
//! `channels`'.

use rekindle_governance::validate::bounds;
use rekindle_ipc::protocol::{CommunityRequest, IpcResponse};
use rekindle_types::game_server;
use rekindle_types::governance::ExclusionGroupEdit;
use rekindle_types::invite::InviteLink;
use rekindle_types::key_format::{self, MAX_NOTE_LEN};
use rekindle_types::member::MAX_TIMEOUT_SECONDS;

/// The longest exclusion-group slug in bytes (the Host normalizes it to at
/// most 32 characters of ASCII).
const MAX_EXCLUSION_GROUP_LEN: usize = 32;

/// The longest community policy text, in characters (architecture §10.7).
const MAX_POLICY_TEXT_CHARS: usize = 4096;

use crate::host::adapters::governance::community_pictures::{
    is_picture_hash, MAX_UPLOAD_BYTES as MAX_PICTURE_UPLOAD_BYTES,
};

use super::{community, key_scope, name, opt, pseudonym, reject, text, Checked};

/// A community request's fields.
pub(super) fn check(request: &CommunityRequest) -> Checked {
    match request {
        CommunityRequest::CommunityList | CommunityRequest::CommunityViews => Ok(()),
        CommunityRequest::GameServerList { community: key }
        | CommunityRequest::CommunityPolicyGet { community: key } => community(key),
        CommunityRequest::CommunityUpdateProfile {
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
        CommunityRequest::CommunitySetAvatar {
            community: key,
            image,
            kind: _,
        }
        | CommunityRequest::CommunitySetBanner {
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
        CommunityRequest::CommunityAvatarGet { community: key, hash } => {
            community(key)?;
            if is_picture_hash(hash) {
                Ok(())
            } else {
                Err(IpcResponse::error(400, "a picture hash is 64 lowercase hex digits"))
            }
        }
        CommunityRequest::CommunityPolicySet {
            community: key,
            policy_text,
            max_joins_per_interval,
            join_interval_seconds,
        } => {
            community(key)?;
            if *max_joins_per_interval == 0 || *join_interval_seconds == 0 {
                return Err(IpcResponse::error(400, "the raid thresholds must be above 0"));
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
        CommunityRequest::EventList { .. }
        | CommunityRequest::EventAttendees { .. }
        | CommunityRequest::EventCreate { .. }
        | CommunityRequest::EventEdit { .. }
        | CommunityRequest::EventCancel { .. }
        | CommunityRequest::EventDelete { .. }
        | CommunityRequest::EventRsvp { .. } => super::community_events::check(request),
        CommunityRequest::GameServerAdd {
            community: key,
            game_id,
            label,
            address,
        } => {
            community(key)?;
            name(game_id, "game id")?;
            text(game_id, game_server::MAX_GAME_ID_LEN, "game id")?;
            name(label, "server label")?;
            text(label, game_server::MAX_GAME_SERVER_LABEL_LEN, "server label")?;
            game_server::ServerAddress::parse(address)
                .map(drop)
                .map_err(|e| IpcResponse::error(400, format!("server address: {e}")))
        }
        CommunityRequest::GameServerRemove {
            community: key,
            server_id,
        } => {
            community(key)?;
            if game_server::is_server_id(server_id) {
                Ok(())
            } else {
                Err(IpcResponse::error(400, "server id must be gs_ and 16 hex digits"))
            }
        }
        CommunityRequest::CommunityCreate {
            name: community_name,
            description,
            approval_required: _,
        } => {
            name(community_name, "community")?;
            bounds::description(description)
                .map(drop)
                .map_err(|e| reject("description", &e))
        }
        CommunityRequest::CommunityJoin { invite } => InviteLink::parse(invite).map(drop).map_err(|e| {
            IpcResponse::error(
                400,
                format!(
                    "join requires a full invite link \
                     (rekindle://invite/{{governance_key}}/{{secrets_record_key}}/{{invite_code}}): {e}"
                ),
            )
        }),
        CommunityRequest::CommunityLeave { governance_key } | CommunityRequest::CommunityInfo { governance_key } | CommunityRequest::CommunityMembers { governance_key } | CommunityRequest::CommunityPendingMembers { governance_key } => community(governance_key),
        CommunityRequest::CommunityUpdateInfo {
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
        CommunityRequest::CommunityApprove {
            governance_key,
            member_pseudonym,
        } => {
            community(governance_key)?;
            pseudonym(member_pseudonym, "member pseudonym")
        }
        CommunityRequest::CommunityReject {
            governance_key,
            member_pseudonym,
            reason,
        } => {
            community(governance_key)?;
            pseudonym(member_pseudonym, "member pseudonym")?;
            text(reason, MAX_NOTE_LEN, "reason")
        }
        CommunityRequest::CommunityTransferOwnership {
            governance_key,
            new_owner_pseudonym,
        } => {
            community(governance_key)?;
            pseudonym(new_owner_pseudonym, "new owner pseudonym")
        }

        CommunityRequest::ChannelList { community: c } | CommunityRequest::MekList { community: c } | CommunityRequest::RoleList { community: c } | CommunityRequest::BanList { community: c } | CommunityRequest::InviteList { community: c }
        // Role ids, invite use counts and expiries are governance rules,
        // checked where the op is validated.
        | CommunityRequest::RoleDelete {
            community: c,
            role_id: _,
        } | CommunityRequest::RoleSelfAssign {
            community: c,
            role_id: _,
        } | CommunityRequest::RoleSelfUnassign {
            community: c,
            role_id: _,
        } | CommunityRequest::InviteCreate {
            community: c,
            max_uses: _,
            expires_seconds: _,
        } => community(c),
        CommunityRequest::ChannelCreate { .. } | CommunityRequest::ChannelDelete { .. } | CommunityRequest::ChannelUpdate { .. } | CommunityRequest::ChannelRename { .. } | CommunityRequest::ChannelMove { .. } | CommunityRequest::ChannelReorder { .. } | CommunityRequest::ChannelSetTopic { .. } | CommunityRequest::ChannelSetForumTags { .. } | CommunityRequest::ChannelOverwriteSet { .. } | CommunityRequest::ChannelOverwriteDelete { .. } | CommunityRequest::CategoryCreate { .. } | CommunityRequest::CategoryDelete { .. } | CommunityRequest::CategoryRename { .. } | CommunityRequest::CategoryReorder { .. } | CommunityRequest::ChannelSend { .. } | CommunityRequest::ChannelHistory { .. } => super::channels::check(request),

        CommunityRequest::MekRotate {
            community: c,
            channel: ch,
        } | CommunityRequest::MekRequest {
            community: c,
            channel: ch,
            generation: _,
        } => {
            community(c)?;
            key_scope(ch)
        }

        CommunityRequest::RoleCreate {
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
        CommunityRequest::RoleUpdate {
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
        CommunityRequest::RoleAssign {
            community: c,
            member_pseudonym,
            role_id: _,
        } | CommunityRequest::RoleUnassign {
            community: c,
            member_pseudonym,
            role_id: _,
        } => {
            community(c)?;
            pseudonym(member_pseudonym, "member pseudonym")
        }

        CommunityRequest::Kick {
            community: c,
            target_pseudonym,
        } | CommunityRequest::Unban {
            community: c,
            target_pseudonym,
        } | CommunityRequest::TimeoutRemove {
            community: c,
            target_pseudonym,
        } => {
            community(c)?;
            pseudonym(target_pseudonym, "target pseudonym")
        }
        CommunityRequest::Ban {
            community: c,
            target_pseudonym,
            reason,
        } => {
            community(c)?;
            pseudonym(target_pseudonym, "target pseudonym")?;
            opt(reason.as_deref(), |r| text(r, MAX_NOTE_LEN, "reason"))
        }
        CommunityRequest::Timeout {
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

        CommunityRequest::InviteRevoke {
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

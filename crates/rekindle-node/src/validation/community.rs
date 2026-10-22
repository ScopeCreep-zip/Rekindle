//! A community's requests (plan E3.1): its governance key, names, member
//! pseudonyms, roles, moderation, invites and channel keys; channels are
//! `channels`'.

use rekindle_governance::validate::bounds;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::game_server;
use rekindle_types::governance::ExclusionGroupEdit;
use rekindle_types::invite::InviteLink;
use rekindle_types::key_format::{self, MAX_NOTE_LEN};
use rekindle_types::member::MAX_TIMEOUT_SECONDS;

/// The longest exclusion-group slug in bytes (the Host normalizes it to at
/// most 32 characters of ASCII).
const MAX_EXCLUSION_GROUP_LEN: usize = 32;

use super::{community, key_scope, name, opt, pseudonym, reject, text, Checked};

/// A community request's fields.
pub(super) fn check(request: &IpcRequest) -> Checked {
    match request {
        IpcRequest::CommunityList | IpcRequest::CommunityViews => Ok(()),
        IpcRequest::GameServerList { community: key } => community(key),
        IpcRequest::GameServerAdd {
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
        IpcRequest::GameServerRemove {
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
        IpcRequest::CommunityCreate {
            name: community_name,
            description,
            approval_required: _,
        } => {
            name(community_name, "community")?;
            bounds::description(description)
                .map(drop)
                .map_err(|e| reject("description", &e))
        }
        IpcRequest::CommunityJoin { invite } => InviteLink::parse(invite).map(drop).map_err(|e| {
            IpcResponse::error(
                400,
                format!(
                    "join requires a full invite link \
                     (rekindle://invite/{{governance_key}}/{{secrets_record_key}}/{{invite_code}}): {e}"
                ),
            )
        }),
        IpcRequest::CommunityLeave { governance_key }
        | IpcRequest::CommunityInfo { governance_key }
        | IpcRequest::CommunityMembers { governance_key }
        | IpcRequest::CommunityPendingMembers { governance_key } => community(governance_key),
        IpcRequest::CommunityUpdateInfo {
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
        IpcRequest::CommunityApprove {
            governance_key,
            member_pseudonym,
        } => {
            community(governance_key)?;
            pseudonym(member_pseudonym, "member pseudonym")
        }
        IpcRequest::CommunityReject {
            governance_key,
            member_pseudonym,
            reason,
        } => {
            community(governance_key)?;
            pseudonym(member_pseudonym, "member pseudonym")?;
            text(reason, MAX_NOTE_LEN, "reason")
        }
        IpcRequest::CommunityTransferOwnership {
            governance_key,
            new_owner_pseudonym,
        } => {
            community(governance_key)?;
            pseudonym(new_owner_pseudonym, "new owner pseudonym")
        }

        IpcRequest::ChannelList { community: c }
        | IpcRequest::MekList { community: c }
        | IpcRequest::RoleList { community: c }
        | IpcRequest::BanList { community: c }
        | IpcRequest::InviteList { community: c }
        // Role ids, invite use counts and expiries are governance rules,
        // checked where the op is validated.
        | IpcRequest::RoleDelete {
            community: c,
            role_id: _,
        }
        | IpcRequest::RoleSelfAssign {
            community: c,
            role_id: _,
        }
        | IpcRequest::RoleSelfUnassign {
            community: c,
            role_id: _,
        }
        | IpcRequest::InviteCreate {
            community: c,
            max_uses: _,
            expires_seconds: _,
        } => community(c),
        IpcRequest::ChannelCreate { .. }
        | IpcRequest::ChannelDelete { .. }
        | IpcRequest::ChannelUpdate { .. }
        | IpcRequest::ChannelRename { .. }
        | IpcRequest::ChannelMove { .. }
        | IpcRequest::ChannelReorder { .. }
        | IpcRequest::ChannelSetTopic { .. }
        | IpcRequest::ChannelSetForumTags { .. }
        | IpcRequest::ChannelOverwriteSet { .. }
        | IpcRequest::ChannelOverwriteDelete { .. }
        | IpcRequest::CategoryCreate { .. }
        | IpcRequest::CategoryDelete { .. }
        | IpcRequest::CategoryRename { .. }
        | IpcRequest::CategoryReorder { .. }
        | IpcRequest::ChannelSend { .. }
        | IpcRequest::ChannelHistory { .. } => super::channels::check(request),

        IpcRequest::MekRotate {
            community: c,
            channel: ch,
        }
        | IpcRequest::MekRequest {
            community: c,
            channel: ch,
            generation: _,
        } => {
            community(c)?;
            key_scope(ch)
        }

        IpcRequest::RoleCreate {
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
        IpcRequest::RoleUpdate {
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
        IpcRequest::RoleAssign {
            community: c,
            member_pseudonym,
            role_id: _,
        }
        | IpcRequest::RoleUnassign {
            community: c,
            member_pseudonym,
            role_id: _,
        } => {
            community(c)?;
            pseudonym(member_pseudonym, "member pseudonym")
        }

        IpcRequest::Kick {
            community: c,
            target_pseudonym,
        }
        | IpcRequest::Unban {
            community: c,
            target_pseudonym,
        } => {
            community(c)?;
            pseudonym(target_pseudonym, "target pseudonym")
        }
        IpcRequest::Ban {
            community: c,
            target_pseudonym,
            reason,
        } => {
            community(c)?;
            pseudonym(target_pseudonym, "target pseudonym")?;
            opt(reason.as_deref(), |r| text(r, MAX_NOTE_LEN, "reason"))
        }
        IpcRequest::Timeout {
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

        IpcRequest::InviteRevoke {
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
        other => unreachable!("{} is not a community request", other.name()),
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

//! The audit, game-server, automod, read-marker and block field checks of
//! the community requests (plan E3.1).

use super::{community, name, pseudonym, text, Checked};
use crate::host::adapters::governance::audit::MAX_AUDIT_PAGE;
use rekindle_governance_runtime::audit::NOTIFICATION_LEVELS;
use rekindle_ipc::protocol::IpcResponse;
use rekindle_ipc::protocol::MemberRequest;
use rekindle_ipc::protocol::{ActivityRequest, AuditRequest, UnreadRequest};
use rekindle_types::game_server;

/// The longest channel or message id a read marker names.
const MAX_UNREAD_ID_LEN: usize = 128;

/// A read-marker or unread-count request (plan E3.1 G1).
pub(super) fn unread_fields(request: &UnreadRequest) -> Checked {
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
        UnreadRequest::MembersOnScreen {
            community: key,
            members,
        } => {
            community(key)?;
            if members.len() > rekindle_presence::community::MEMBERS_ON_SCREEN_MAX {
                return Err(IpcResponse::error(
                    400,
                    "more members on screen than the Host inspects",
                ));
            }
            members.iter().try_for_each(|m| pseudonym(m, "member"))
        }
    }
}

/// A community block request (plan E3.1 F3, D26).
pub(super) fn block_fields(request: &MemberRequest) -> Checked {
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

/// The audit-log page bound and the notification level (plan E3.1 F4).
pub(super) fn audit_fields(request: &AuditRequest) -> Checked {
    match request {
        AuditRequest::AuditLog {
            community: key,
            before_timestamp: _,
            limit,
        } => {
            community(key)?;
            if *limit == 0 || *limit > MAX_AUDIT_PAGE {
                return Err(IpcResponse::error(
                    400,
                    format!("invalid limit: must be 1..={MAX_AUDIT_PAGE}"),
                ));
            }
            Ok(())
        }
        AuditRequest::NotificationDefaultSet {
            community: key,
            level,
        } => {
            community(key)?;
            if !NOTIFICATION_LEVELS.contains(&level.as_str()) {
                return Err(IpcResponse::error(
                    400,
                    format!("invalid level: must be one of {NOTIFICATION_LEVELS:?}"),
                ));
            }
            Ok(())
        }
        AuditRequest::NotificationDefaultGet { .. } => Ok(()),
    }
}

/// A game server add's or removal's fields (plan E3.1 H1).
pub(super) fn game_server_fields(request: &ActivityRequest) -> Checked {
    match request {
        ActivityRequest::GameServerAdd {
            community: key,
            game_id,
            label,
            address,
        } => {
            community(key)?;
            name(game_id, "game id")?;
            text(game_id, game_server::MAX_GAME_ID_LEN, "game id")?;
            name(label, "server label")?;
            text(
                label,
                game_server::MAX_GAME_SERVER_LABEL_LEN,
                "server label",
            )?;
            game_server::ServerAddress::parse(address)
                .map(drop)
                .map_err(|e| IpcResponse::error(400, format!("server address: {e}")))
        }
        ActivityRequest::GameServerRemove {
            community: key,
            server_id,
        } => {
            community(key)?;
            if game_server::is_server_id(server_id) {
                Ok(())
            } else {
                Err(IpcResponse::error(
                    400,
                    "server id must be gs_ and 16 hex digits",
                ))
            }
        }
        _ => Ok(()),
    }
}

/// An automod rule set's or delete's fields.
pub(super) fn automod_fields(request: &MemberRequest) -> Checked {
    match request {
        MemberRequest::AutomodRuleSet {
            community: key,
            rule_id,
            name,
            ..
        } => {
            community(key)?;
            if let Some(id) = rule_id {
                rule_id_field(id)?;
            }
            if name.trim().is_empty() {
                return Err(IpcResponse::error(400, "an automod rule needs a name"));
            }
            Ok(())
        }
        MemberRequest::AutomodRuleDelete {
            community: key,
            rule_id,
        } => {
            community(key)?;
            rule_id_field(rule_id)
        }
        _ => Ok(()),
    }
}

/// A role id: the full 16 bytes as 32 hex characters (plan E3.2 S9).
pub(super) fn role_id_field(id: &str) -> Result<(), IpcResponse> {
    rekindle_types::id::RoleId::from_hex(id)
        .map(|_| ())
        .ok_or_else(|| IpcResponse::error(400, "invalid role id: must be 32 hex characters"))
}

/// An automod rule id: 16 bytes of hex.
fn rule_id_field(id: &str) -> Result<(), IpcResponse> {
    match hex::decode(id) {
        Ok(bytes) if bytes.len() == 16 => Ok(()),
        _ => Err(IpcResponse::error(
            400,
            "invalid rule id: must be 32 hex characters",
        )),
    }
}

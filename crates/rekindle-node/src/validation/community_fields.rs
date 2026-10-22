//! The audit, game-server and automod field checks of the community
//! requests (plan E3.1).

use super::{community, name, text, Checked};
use crate::host::adapters::governance::audit::MAX_AUDIT_PAGE;
use rekindle_governance_runtime::audit::NOTIFICATION_LEVELS;
use rekindle_ipc::protocol::IpcResponse;
use rekindle_ipc::protocol::MemberRequest;
use rekindle_ipc::protocol::{ActivityRequest, AuditRequest};
use rekindle_types::game_server;

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
        _ => Ok(()),
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

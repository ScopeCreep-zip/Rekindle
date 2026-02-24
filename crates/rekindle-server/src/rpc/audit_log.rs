use std::sync::Arc;

use rekindle_protocol::dht::community::permissions;
use rekindle_protocol::messaging::envelope::{AuditLogEntryDto, CommunityResponse};
use rusqlite::params;

use crate::server_state::ServerState;

use super::permissions::{check_permission, verify_membership};

pub(super) fn handle_get_audit_log(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    before_timestamp: Option<u64>,
    limit: u32,
) -> CommunityResponse {
    {
        let hosted = state.hosted.read();
        let Some(community) = hosted.get(community_id) else {
            return CommunityResponse::Error {
                code: 404,
                message: "community not found".into(),
            };
        };
        if let Err(e) = verify_membership(community, sender_pseudonym) {
            return e;
        }
        if let Err(e) = check_permission(
            community,
            sender_pseudonym,
            permissions::VIEW_AUDIT_LOG,
        ) {
            return e;
        }
    }

    let capped_limit = limit.min(100);
    let db = crate::db_helpers::lock_db(&state.db);

    let entries: Vec<AuditLogEntryDto> = if let Some(before) = before_timestamp {
        let before_i64 = i64::try_from(before).unwrap_or(i64::MAX);
        let mut stmt = match db.prepare(
            "SELECT action, actor_pseudonym, target, details, timestamp \
             FROM server_audit_log WHERE community_id = ? AND timestamp < ? \
             ORDER BY timestamp DESC LIMIT ?",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "failed to prepare audit log query");
                return CommunityResponse::Error {
                    code: 500,
                    message: "failed to query audit log".into(),
                };
            }
        };
        stmt.query_map(params![community_id, before_i64, capped_limit], |row| {
            Ok(AuditLogEntryDto {
                action: row.get(0)?,
                actor_pseudonym: row.get(1)?,
                target: row.get(2)?,
                details: row.get(3)?,
                timestamp: row.get::<_, i64>(4).map(i64::cast_unsigned)?,
            })
        })
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    } else {
        let mut stmt = match db.prepare(
            "SELECT action, actor_pseudonym, target, details, timestamp \
             FROM server_audit_log WHERE community_id = ? \
             ORDER BY timestamp DESC LIMIT ?",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "failed to prepare audit log query");
                return CommunityResponse::Error {
                    code: 500,
                    message: "failed to query audit log".into(),
                };
            }
        };
        stmt.query_map(params![community_id, capped_limit], |row| {
            Ok(AuditLogEntryDto {
                action: row.get(0)?,
                actor_pseudonym: row.get(1)?,
                target: row.get(2)?,
                details: row.get(3)?,
                timestamp: row.get::<_, i64>(4).map(i64::cast_unsigned)?,
            })
        })
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    };

    CommunityResponse::AuditLog { entries }
}

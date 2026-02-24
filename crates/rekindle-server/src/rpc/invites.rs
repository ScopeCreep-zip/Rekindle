use std::sync::Arc;

use rekindle_protocol::dht::community::permissions;
use rekindle_protocol::messaging::envelope::{CommunityBroadcast, CommunityResponse, InviteDto};
use rusqlite::params;

use crate::audit;
use crate::server_state::ServerState;

use super::broadcast::broadcast_to_members;
use super::permissions::{check_permission, verify_membership};

pub(super) fn handle_create_invite(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    max_uses: Option<u32>,
    expires_in_seconds: Option<u64>,
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
        if let Err(e) =
            check_permission(community, sender_pseudonym, permissions::CREATE_INSTANT_INVITE)
        {
            return e;
        }
    }

    // Generate a short, URL-safe invite code and sign it with the server identity
    let code = crate::invite_util::generate_invite_code();

    let now = rekindle_utils::timestamp_secs();
    let expires_at = expires_in_seconds.map(|s| now + s);

    // Sign the invite code with the server's identity key so clients can verify
    // the invite was issued by this server (not forged).
    let signature = state.identity.sign(code.as_bytes());

    {
        let db = crate::db_helpers::lock_db(&state.db);

        let now_i64 = i64::try_from(now).unwrap_or(i64::MAX);
        let expires_at_i64 = expires_at.map(|e| i64::try_from(e).unwrap_or(i64::MAX));
        let max_uses_i64 = max_uses.map(i64::from);

        if let Err(e) = db.execute(
            "INSERT INTO server_invites (code, community_id, created_by, max_uses, expires_at, created_at) VALUES (?,?,?,?,?,?)",
            params![code, community_id, sender_pseudonym, max_uses_i64, expires_at_i64, now_i64],
        ) {
            tracing::error!(error = %e, "failed to insert invite into DB");
            return CommunityResponse::Error {
                code: 500,
                message: "failed to create invite".into(),
            };
        }
    } // DB lock dropped before audit/broadcast

    tracing::info!(community = %community_id, invite = %code, "invite created");
    audit::log_action(state, community_id, audit::AuditAction::CreateInvite, sender_pseudonym, None, Some(&code));
    broadcast_to_members(
        state,
        community_id,
        "",
        &CommunityBroadcast::InviteCreated {
            community_id: community_id.to_string(),
            code: code.clone(),
            created_by: sender_pseudonym.to_string(),
            max_uses,
            uses: 0,
            expires_at,
            created_at: now,
        },
    );
    CommunityResponse::InviteCreated {
        code,
        signature: hex::encode(signature.to_bytes()),
    }
}

pub(super) fn handle_revoke_invite(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    code: &str,
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
        // Revoking requires MANAGE_COMMUNITY (not just CREATE_INSTANT_INVITE)
        if let Err(e) =
            check_permission(community, sender_pseudonym, permissions::MANAGE_COMMUNITY)
        {
            return e;
        }
    }

    {
        let db = crate::db_helpers::lock_db(&state.db);

        let deleted = db
            .execute(
                "DELETE FROM server_invites WHERE code = ? AND community_id = ?",
                params![code, community_id],
            )
            .unwrap_or(0);

        if deleted == 0 {
            return CommunityResponse::Error {
                code: 404,
                message: "invite not found".into(),
            };
        }
    } // DB lock dropped before audit/broadcast

    tracing::info!(community = %community_id, invite = %code, "invite revoked");
    audit::log_action(state, community_id, audit::AuditAction::RevokeInvite, sender_pseudonym, None, Some(code));
    broadcast_to_members(
        state,
        community_id,
        "",
        &CommunityBroadcast::InviteRevoked {
            community_id: community_id.to_string(),
            code: code.to_string(),
        },
    );
    CommunityResponse::Ok
}

pub(super) fn handle_list_invites(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
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
        if let Err(e) =
            check_permission(community, sender_pseudonym, permissions::MANAGE_COMMUNITY)
        {
            return e;
        }
    }

    let db = crate::db_helpers::lock_db(&state.db);

    let invites = db
        .prepare(
            "SELECT code, created_by, max_uses, uses, expires_at, created_at \
             FROM server_invites WHERE community_id = ? ORDER BY created_at DESC",
        )
        .and_then(|mut stmt| {
            let rows = stmt.query_map(params![community_id], |row| {
                let max_uses: Option<i64> = row.get(2)?;
                let uses: i64 = row.get(3)?;
                let expires_at: Option<i64> = row.get(4)?;
                let created_at: i64 = row.get(5)?;
                Ok(InviteDto {
                    code: row.get(0)?,
                    created_by: row.get(1)?,
                    max_uses: max_uses.map(|v| u32::try_from(v).unwrap_or(u32::MAX)),
                    uses: u32::try_from(uses).unwrap_or(u32::MAX),
                    expires_at: expires_at.map(|v| v.max(0).cast_unsigned()),
                    created_at: created_at.max(0).cast_unsigned(),
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>()
        })
        .unwrap_or_default();

    CommunityResponse::InviteList { invites }
}

/// Validate an invite code against the `server_invites` table.
///
/// Checks existence, expiry, and usage limits. On success, increments the
/// `uses` counter and returns `Ok(())`. On failure, returns an error response.
pub(super) fn validate_and_consume_invite(
    state: &Arc<ServerState>,
    community_id: &str,
    code: &str,
) -> Result<(), CommunityResponse> {
    let new_use_count = {
        let db = crate::db_helpers::lock_db(&state.db);

        let row: Result<(i64, Option<i64>, Option<i64>), _> = db.query_row(
            "SELECT uses, max_uses, expires_at FROM server_invites WHERE code = ? AND community_id = ?",
            params![code, community_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        );

        let (uses, max_uses, expires_at) = row.map_err(|_| CommunityResponse::Error {
            code: 404,
            message: "invalid invite code".into(),
        })?;

        // Check expiry
        if let Some(exp) = expires_at {
            let now = i64::try_from(rekindle_utils::timestamp_secs()).unwrap_or(i64::MAX);
            if now > exp {
                return Err(CommunityResponse::Error {
                    code: 410,
                    message: "invite has expired".into(),
                });
            }
        }

        // Check usage limit
        if let Some(max) = max_uses {
            if uses >= max {
                return Err(CommunityResponse::Error {
                    code: 410,
                    message: "invite has reached its usage limit".into(),
                });
            }
        }

        // Increment uses — check return value to detect concurrent revocation
        let updated = db
            .execute(
                "UPDATE server_invites SET uses = uses + 1 WHERE code = ? AND community_id = ?",
                params![code, community_id],
            )
            .map_err(|e| CommunityResponse::Error {
                code: 500,
                message: format!("failed to consume invite: {e}"),
            })?;

        if updated == 0 {
            return Err(CommunityResponse::Error {
                code: 410,
                message: "invite was revoked".into(),
            });
        }

        u32::try_from(uses + 1).unwrap_or(u32::MAX)
    }; // DB lock dropped before broadcast

    broadcast_to_members(
        state,
        community_id,
        "",
        &CommunityBroadcast::InviteUsed {
            community_id: community_id.to_string(),
            code: code.to_string(),
            new_use_count,
        },
    );

    Ok(())
}

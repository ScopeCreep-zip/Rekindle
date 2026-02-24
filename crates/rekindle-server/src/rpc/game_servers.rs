use std::sync::Arc;

use rekindle_protocol::dht::community::permissions;
use rekindle_protocol::messaging::envelope::{CommunityBroadcast, CommunityResponse, GameServerDto};
use rusqlite::params;

use crate::audit;
use crate::server_state::ServerState;

use super::broadcast::broadcast_to_members;
use super::permissions::{check_permission, verify_membership};

// ---------------------------------------------------------------------------
// Game server favorites
// ---------------------------------------------------------------------------

pub(super) fn handle_add_game_server(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    game_id: &str,
    label: &str,
    address: &str,
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

    let now = rekindle_utils::timestamp_secs_i64();
    let server_id = format!("gs_{}", hex::encode(super::rand_bytes(8)));

    let db = crate::db_helpers::lock_db(&state.db);
    if let Err(e) = db.execute(
        "INSERT INTO server_game_servers (community_id, id, game_id, label, address, added_by, created_at) VALUES (?,?,?,?,?,?,?)",
        params![community_id, server_id, game_id, label, address, sender_pseudonym, now],
    ) {
        tracing::error!(error = %e, "failed to add game server");
        return CommunityResponse::Error {
            code: 500,
            message: "failed to add game server".into(),
        };
    }
    drop(db);

    let server = GameServerDto {
        id: server_id,
        game_id: game_id.to_string(),
        label: label.to_string(),
        address: address.to_string(),
        added_by: sender_pseudonym.to_string(),
        created_at: now.try_into().unwrap_or(0),
    };

    audit::log_action(
        state,
        community_id,
        audit::AuditAction::AddGameServer,
        sender_pseudonym,
        Some(&server.id),
        Some(label),
    );

    broadcast_to_members(
        state,
        community_id,
        "",
        &CommunityBroadcast::GameServerAdded {
            community_id: community_id.to_string(),
            server: server.clone(),
        },
    );

    CommunityResponse::GameServerList {
        servers: vec![server],
    }
}

pub(super) fn handle_remove_game_server(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    server_id: &str,
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
    let deleted = db.execute(
        "DELETE FROM server_game_servers WHERE community_id = ? AND id = ?",
        params![community_id, server_id],
    );
    drop(db);

    match deleted {
        Ok(0) => {
            return CommunityResponse::Error {
                code: 404,
                message: "game server not found".into(),
            };
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to remove game server");
            return CommunityResponse::Error {
                code: 500,
                message: "failed to remove game server".into(),
            };
        }
        Ok(_) => {}
    }

    audit::log_action(
        state,
        community_id,
        audit::AuditAction::RemoveGameServer,
        sender_pseudonym,
        Some(server_id),
        None,
    );

    broadcast_to_members(
        state,
        community_id,
        "",
        &CommunityBroadcast::GameServerRemoved {
            community_id: community_id.to_string(),
            server_id: server_id.to_string(),
        },
    );

    CommunityResponse::Ok
}

pub(super) fn handle_get_game_servers(
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
    }

    let db = crate::db_helpers::lock_db(&state.db);

    let servers = {
        let mut stmt = match db.prepare(
            "SELECT id, game_id, label, address, added_by, created_at FROM server_game_servers WHERE community_id = ? ORDER BY created_at ASC",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "failed to query game servers");
                return CommunityResponse::Error {
                    code: 500,
                    message: "failed to query game servers".into(),
                };
            }
        };
        let rows = stmt.query_map(params![community_id], |row| {
            let created_at_i64: i64 = row.get(5)?;
            Ok(GameServerDto {
                id: row.get(0)?,
                game_id: row.get(1)?,
                label: row.get(2)?,
                address: row.get(3)?,
                added_by: row.get(4)?,
                created_at: created_at_i64.cast_unsigned(),
            })
        });
        match rows {
            Ok(r) => r.filter_map(Result::ok).collect(),
            Err(e) => {
                tracing::error!(error = %e, "failed to iterate game servers");
                return CommunityResponse::Error {
                    code: 500,
                    message: "failed to query game servers".into(),
                };
            }
        }
    };

    CommunityResponse::GameServerList { servers }
}

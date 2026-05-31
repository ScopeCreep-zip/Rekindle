//! Phase 23.C — community game-servers runtime orchestration lifted
//! from `commands/community/game_servers.rs`. Hosts `add_game_server`
//! (random server_id + DTO build + gossip `GameServerAdded`) and
//! `get_game_servers` (SQLite scan of cached entries).

use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_protocol::dht::community::permissions_v2::Permissions;

use crate::channels::community_channel::GameServerInfoDto;
use crate::commands::community::helpers::{random_nonce, require_permission};
use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;

pub fn add_game_server_inner(
    state: &SharedState,
    community_id: &str,
    game_id: String,
    label: String,
    address: String,
) -> Result<String, String> {
    require_permission(state, community_id, Permissions::MANAGE_CHANNELS)?;
    let server_id = format!("gs_{}", hex::encode(random_nonce(8)));
    let added_by = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_default()
    };
    let server = GameServerInfoDto {
        id: server_id.clone(),
        game_id,
        label,
        address,
        added_by,
        created_at: rekindle_utils::timestamp_secs(),
    };

    crate::services::community::send_to_mesh(
        state,
        community_id,
        &CommunityEnvelope::Control(ControlPayload::GameServerAdded {
            server: server.clone(),
        }),
    )?;

    Ok(server_id)
}

pub async fn get_game_servers_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: String,
) -> Result<Vec<GameServerInfoDto>, String> {
    require_permission(state, &community_id, Permissions::VIEW_CHANNEL)?;
    let owner_key = state_helpers::current_owner_key(state)?;
    db_call(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, game_id, label, address, added_by, created_at FROM game_servers \
             WHERE owner_key = ?1 AND community_id = ?2 \
             ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![owner_key, community_id], |row| {
            Ok(GameServerInfoDto {
                id: row.get(0)?,
                game_id: row.get(1)?,
                label: row.get(2)?,
                address: row.get(3)?,
                added_by: row.get(4)?,
                created_at: row.get::<_, i64>(5).unwrap_or(0).cast_unsigned(),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
    })
    .await
}

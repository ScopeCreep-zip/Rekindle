use tauri::State;

use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_protocol::dht::community::permissions_v2::Permissions;

use super::helpers::{random_nonce, require_permission};

pub use crate::channels::community_channel::GameServerInfoDto;

#[tauri::command]
pub async fn add_game_server(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    game_id: String,
    label: String,
    address: String,
) -> Result<String, String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_CHANNELS)?;
    let _ = pool;
    let server_id = format!("gs_{}", hex::encode(random_nonce(8)));
    let added_by = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
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
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::GameServerAdded {
            server: serde_json::to_value(&server).map_err(|e| format!("serialize server: {e}"))?,
        }),
    )?;

    Ok(server_id)
}

#[tauri::command]
pub async fn remove_game_server(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    server_id: String,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_CHANNELS)?;
    let _ = pool;
    crate::services::community::send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::GameServerRemoved { server_id }),
    )
}

#[tauri::command]
pub async fn get_game_servers(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
) -> Result<Vec<GameServerInfoDto>, String> {
    require_permission(state.inner(), &community_id, Permissions::VIEW_CHANNEL)?;
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    db_call(pool.inner(), move |conn| {
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

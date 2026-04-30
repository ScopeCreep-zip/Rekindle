use tauri::State;

use crate::db::{self, DbPool};
use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;
use rekindle_protocol::dht::community::envelope::CommunityEnvelope;

use super::types::MemberDto;

#[tauri::command]
pub async fn send_channel_typing(
    community_id: String,
    channel_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let _ = pool;

    let pseudonym_key = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .ok_or("no pseudonym key")?
    };

    let envelope = CommunityEnvelope::TypingIndicator {
        channel_id,
        pseudonym_key,
    };
    crate::services::community::send_to_mesh(state.inner(), &community_id, &envelope)
}

#[tauri::command]
pub async fn update_community_presence(
    community_id: String,
    status: String,
    game_name: Option<String>,
    game_id: Option<u32>,
    elapsed_seconds: Option<u32>,
    server_address: Option<String>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    let _ = pool;

    let pseudonym_key = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .ok_or("no pseudonym key")?
    };

    let game_info =
        game_name.map(
            |name| rekindle_protocol::dht::community::envelope::PresenceGameInfo {
                game_name: name,
                game_id,
                elapsed_seconds,
                server_address: server_address.clone(),
            },
        );

    let envelope = CommunityEnvelope::PresenceUpdate {
        pseudonym_key,
        status,
        game_info,
        route_blob: crate::state_helpers::our_route_blob(state.inner()),
    };
    crate::services::community::send_to_mesh(state.inner(), &community_id, &envelope)
}

#[tauri::command]
pub async fn get_community_members(
    community_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<MemberDto>, String> {
    let my_pseudonym = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
    };
    let my_status =
        state_helpers::identity_status(state.inner()).unwrap_or(crate::state::UserStatus::Online);

    let (role_defs, online_statuses) = {
        let communities = state.communities.read();
        communities.get(&community_id).map_or_else(
            || (Vec::new(), std::collections::HashMap::new()),
            |c| {
                (
                    c.roles.clone(),
                    c.gossip
                        .as_ref()
                        .map(|g| {
                            g.online_members
                                .iter()
                                .map(|(pk, member)| (pk.clone(), member.status.clone()))
                                .collect::<std::collections::HashMap<_, _>>()
                        })
                        .unwrap_or_default(),
                )
            },
        )
    };

    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let community_id_clone = community_id.clone();
    let members = db_call(pool.inner(), move |conn| {
        let mut stmt = conn.prepare(
            "SELECT pseudonym_key, display_name, role_ids, timeout_until FROM community_members \
                 WHERE owner_key = ? AND community_id = ? ORDER BY display_name",
        )?;

        let rows = stmt.query_map(rusqlite::params![owner_key, community_id_clone], |row| {
            let pseudonym_key = db::get_str(row, "pseudonym_key");
            let status_str = if my_pseudonym.as_deref() == Some(&pseudonym_key) {
                match my_status {
                    crate::state::UserStatus::Online => "online",
                    crate::state::UserStatus::Away => "away",
                    crate::state::UserStatus::Busy => "busy",
                    crate::state::UserStatus::Offline | crate::state::UserStatus::Invisible => {
                        "offline"
                    }
                }
            } else {
                online_statuses
                    .get(&pseudonym_key)
                    .map_or("offline", String::as_str)
            };

            let role_ids_json = db::get_str(row, "role_ids");
            let role_ids: Vec<u32> =
                serde_json::from_str(&role_ids_json).unwrap_or_else(|_| vec![0, 1]);
            let display_role = crate::state::display_role_name(&role_ids, &role_defs);
            let timeout_until: Option<u64> = row
                .get::<_, Option<i64>>("timeout_until")
                .ok()
                .flatten()
                .map(i64::cast_unsigned);

            Ok(MemberDto {
                pseudonym_key,
                display_name: db::get_str(row, "display_name"),
                role_ids,
                display_role,
                status: status_str.to_string(),
                timeout_until,
            })
        })?;

        let mut members = Vec::new();
        for row in rows {
            members.push(row?);
        }
        Ok(members)
    })
    .await?;

    Ok(members)
}

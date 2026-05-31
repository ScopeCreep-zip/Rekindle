//! Phase 23.C — presence-handler Tauri-runtime orchestration lifted
//! from `commands/community/presence.rs`. Hosts the four orchestrators
//! (`send_channel_typing_inner`, `update_community_presence_inner`,
//! `get_community_members_inner`, `update_community_profile_inner`).

use rekindle_protocol::dht::community::envelope::CommunityEnvelope;

use crate::db::{self, DbPool};
use crate::db_helpers::db_call;
use crate::services::community_profile_validation::validate_profile;
use crate::state::SharedState;
use crate::state_helpers;

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberDto {
    pub pseudonym_key: String,
    pub display_name: String,
    pub role_ids: Vec<u32>,
    pub display_role: String,
    pub status: String,
    pub timeout_until: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pronouns: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme_color: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub badges: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banner_ref: Option<String>,
}

pub fn send_channel_typing_inner(
    state: &SharedState,
    community_id: &str,
    channel_id: String,
) -> Result<(), String> {
    let pseudonym_key = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .ok_or("no pseudonym key")?
    };

    let envelope = CommunityEnvelope::TypingIndicator {
        channel_id,
        pseudonym_key,
    };
    crate::services::community::send_to_mesh(state, community_id, &envelope)
}

#[allow(
    clippy::too_many_arguments,
    reason = "Tauri command surface — matches PresenceUpdate envelope shape"
)]
pub async fn update_community_presence_inner(
    state: &SharedState,
    community_id: String,
    status: String,
    game_name: Option<String>,
    game_id: Option<u32>,
    elapsed_seconds: Option<u32>,
    server_address: Option<String>,
) -> Result<(), String> {
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
        route_blob: state_helpers::our_route_blob(state),
    };
    let gossip_result = crate::services::community::send_to_mesh(state, &community_id, &envelope);
    crate::services::community::write_our_presence(state, &community_id).await;
    gossip_result
}

pub async fn get_community_members_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: String,
) -> Result<Vec<MemberDto>, String> {
    let my_pseudonym = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
    };
    let my_status =
        state_helpers::identity_status(state).unwrap_or(crate::state::UserStatus::Online);

    let (role_defs, online_statuses, member_profiles, my_profile) = {
        let communities = state.communities.read();
        communities.get(&community_id).map_or_else(
            || {
                (
                    Vec::new(),
                    std::collections::HashMap::new(),
                    std::collections::HashMap::new(),
                    None,
                )
            },
            |c| {
                let online = c
                    .gossip
                    .as_ref()
                    .map(|g| {
                        g.online_members
                            .iter()
                            .map(|(pk, member)| (pk.clone(), member.status.clone()))
                            .collect::<std::collections::HashMap<_, _>>()
                    })
                    .unwrap_or_default();
                let mine = crate::state::MemberProfileSnapshot {
                    display_name: state
                        .identity
                        .read()
                        .as_ref()
                        .map(|id| id.display_name.clone()),
                    bio: c.my_bio.clone(),
                    pronouns: c.my_pronouns.clone(),
                    theme_color: c.my_theme_color,
                    badges: c.my_badges.clone(),
                    avatar_ref: c.my_avatar_ref.clone(),
                    banner_ref: c.my_banner_ref.clone(),
                };
                (
                    c.roles.clone(),
                    online,
                    c.member_profiles.clone(),
                    Some(mine),
                )
            },
        )
    };

    let owner_key = state_helpers::current_owner_key(state)?;
    let community_id_clone = community_id.clone();
    let members = db_call(pool, move |conn| {
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

            let profile = if my_pseudonym.as_deref() == Some(&pseudonym_key) {
                my_profile.clone()
            } else {
                member_profiles.get(&pseudonym_key).cloned()
            };
            let snap = profile.unwrap_or_default();

            Ok(MemberDto {
                pseudonym_key,
                display_name: db::get_str(row, "display_name"),
                role_ids,
                display_role,
                status: status_str.to_string(),
                timeout_until,
                bio: snap.bio,
                pronouns: snap.pronouns,
                theme_color: snap.theme_color,
                badges: snap.badges,
                avatar_ref: snap.avatar_ref,
                banner_ref: snap.banner_ref,
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

#[allow(
    clippy::too_many_arguments,
    reason = "Tauri command surface — matches per-community profile fields"
)]
pub async fn update_community_profile_inner(
    state: &SharedState,
    community_id: String,
    bio: Option<String>,
    pronouns: Option<String>,
    theme_color: Option<u32>,
    badges: Vec<String>,
    avatar_ref: Option<String>,
    banner_ref: Option<String>,
) -> Result<(), String> {
    validate_profile(
        bio.as_deref(),
        pronouns.as_deref(),
        &badges,
        avatar_ref.as_deref(),
        banner_ref.as_deref(),
    )?;
    {
        let mut communities = state.communities.write();
        let community = communities
            .get_mut(&community_id)
            .ok_or_else(|| "unknown community".to_string())?;
        community.my_bio = bio;
        community.my_pronouns = pronouns;
        community.my_theme_color = theme_color;
        community.my_badges = badges;
        community.my_avatar_ref = avatar_ref;
        community.my_banner_ref = banner_ref;
    }

    if let Err(e) =
        crate::services::community::presence::presence_poll_tick_public(state, &community_id).await
    {
        tracing::debug!(
            community = %community_id,
            error = %e,
            "profile-update presence nudge skipped",
        );
    }
    Ok(())
}

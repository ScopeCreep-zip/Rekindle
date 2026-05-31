use tauri::State;

use crate::db::DbPool;
use crate::services::community_presence_runtime::{
    get_community_members_inner, send_channel_typing_inner, update_community_presence_inner,
    update_community_profile_inner,
};
use crate::state::SharedState;

use crate::services::community_presence_runtime::MemberDto;

#[tauri::command]
pub async fn send_channel_typing(
    community_id: String,
    channel_id: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    send_channel_typing_inner(state.inner(), &community_id, channel_id)
}

#[tauri::command]
#[allow(
    clippy::too_many_arguments,
    reason = "Tauri command surface — matches PresenceUpdate envelope shape"
)]
pub async fn update_community_presence(
    community_id: String,
    status: String,
    game_name: Option<String>,
    game_id: Option<u32>,
    elapsed_seconds: Option<u32>,
    server_address: Option<String>,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    update_community_presence_inner(
        state.inner(),
        community_id,
        status,
        game_name,
        game_id,
        elapsed_seconds,
        server_address,
    )
    .await
}

#[tauri::command]
pub async fn get_community_members(
    community_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<MemberDto>, String> {
    get_community_members_inner(state.inner(), pool.inner(), community_id).await
}

#[tauri::command]
#[allow(
    clippy::too_many_arguments,
    reason = "Tauri command surface — matches per-community profile fields"
)]
pub async fn update_community_profile(
    community_id: String,
    bio: Option<String>,
    pronouns: Option<String>,
    theme_color: Option<u32>,
    badges: Vec<String>,
    avatar_ref: Option<String>,
    banner_ref: Option<String>,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    update_community_profile_inner(
        state.inner(),
        community_id,
        bio,
        pronouns,
        theme_color,
        badges,
        avatar_ref,
        banner_ref,
    )
    .await
}

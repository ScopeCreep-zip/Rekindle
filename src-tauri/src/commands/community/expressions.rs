use rekindle_ipc::protocol::ExpressionRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::community::ExpressionInfo;
use rekindle_types::dto::{CommandError, Done};
use rekindle_types::image_kind::ImageKind;
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

/// Upload a custom emoji (the Host's, plan E3.1 E3c).
#[tauri::command]
pub async fn upload_emoji(
    state: State<'_, SharedState>,
    community_id: String,
    name: String,
    bytes: Vec<u8>,
    kind: ImageKind,
) -> Result<Done<String>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Expressions(
            ExpressionRequest::EmojiUpload {
                community: community_id,
                name,
                bytes,
                kind,
            },
        )),
    )
    .await
}

/// Upload a custom sticker (the Host's, plan E3.1 E3c).
#[tauri::command]
pub async fn upload_sticker(
    state: State<'_, SharedState>,
    community_id: String,
    name: String,
    bytes: Vec<u8>,
    kind: ImageKind,
    tags: Option<Vec<String>>,
) -> Result<Done<String>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Expressions(
            ExpressionRequest::StickerUpload {
                community: community_id,
                name,
                bytes,
                kind,
                tags: tags.unwrap_or_default(),
            },
        )),
    )
    .await
}

#[tauri::command]
pub async fn upload_soundboard_sound(
    state: State<'_, SharedState>,
    community_id: String,
    name: String,
    bytes: Vec<u8>,
    tags: Option<Vec<String>>,
    duration_seconds: f32,
    volume: f32,
    emoji: Option<String>,
) -> Result<String, String> {
    crate::services::community::upload_soundboard_sound(
        state.inner(),
        &community_id,
        &name,
        bytes,
        tags.unwrap_or_default(),
        duration_seconds,
        volume,
        emoji,
    )
    .await
}

#[tauri::command]
pub async fn play_soundboard(
    state: State<'_, SharedState>,
    community_id: String,
    channel_id: String,
    expression_id: String,
) -> Result<(), String> {
    crate::services::community::play_soundboard(
        state.inner(),
        &community_id,
        &channel_id,
        &expression_id,
    )
}

/// Delete a custom expression (the Host's, plan E3.1 E3c).
#[tauri::command]
pub async fn delete_emoji(
    state: State<'_, SharedState>,
    community_id: String,
    expression_id: String,
) -> Result<Done<()>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Expressions(
            ExpressionRequest::ExpressionDelete {
                community: community_id,
                expression_id,
            },
        )),
    )
    .await
}

/// The community's expressions (the Host's, plan E3.1 E3c).
#[tauri::command]
pub async fn list_expressions(
    state: State<'_, SharedState>,
    community_id: String,
) -> Result<Done<Vec<ExpressionInfo>>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Expressions(
            ExpressionRequest::Expressions {
                community: community_id,
            },
        )),
    )
    .await
}

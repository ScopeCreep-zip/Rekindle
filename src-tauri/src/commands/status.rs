//! Our status, message, name and avatar are Host calls (plan E1.1): the
//! Host owns our presence and profile and publishes every change.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::presence::OwnPresenceView;
use rekindle_types::dto::{CommandError, Done};
use rekindle_types::image_kind::ImageKind;
use rekindle_types::presence::SessionStatus;
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

/// Set our status (`IpcRequest::PresenceSet`); "Appear Offline" is
/// `invisible`.
#[tauri::command]
pub async fn set_status(
    status: SessionStatus,
    state: State<'_, SharedState>,
) -> Result<Done<OwnPresenceView>, CommandError> {
    host_call::call(&state, IpcRequest::PresenceSet { status }).await
}

/// Change the name we show (`IpcRequest::ProfileSetNickname`).
#[tauri::command]
pub async fn set_nickname(
    nickname: String,
    state: State<'_, SharedState>,
) -> Result<Done<String>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::ProfileSetNickname {
            display_name: nickname,
        },
    )
    .await
}

/// Change our avatar (`IpcRequest::ProfileSetAvatar`): `kind` is the file
/// picker's MIME type; the Host decodes the picture as that kind only and
/// makes it a 128 px WebP.
#[tauri::command]
pub async fn set_avatar(
    avatar_data: Vec<u8>,
    kind: ImageKind,
    state: State<'_, SharedState>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::ProfileSetAvatar {
            image: avatar_data,
            kind,
        },
    )
    .await
}

/// Our own or a friend's avatar as WebP bytes (`IpcRequest::AvatarGet`).
#[tauri::command]
pub async fn get_avatar(
    public_key: String,
    state: State<'_, SharedState>,
) -> Result<Done<Option<Vec<u8>>>, CommandError> {
    host_call::call(&state, IpcRequest::AvatarGet { public_key }).await
}

/// Set our status message (`IpcRequest::PresenceSetMessage`).
#[tauri::command]
pub async fn set_status_message(
    message: String,
    state: State<'_, SharedState>,
) -> Result<Done<OwnPresenceView>, CommandError> {
    host_call::call(&state, IpcRequest::PresenceSetMessage { message }).await
}

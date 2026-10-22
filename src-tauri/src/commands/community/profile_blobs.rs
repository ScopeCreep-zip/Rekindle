//! A community's avatar and banner pictures (plan E3.1 D4): Host calls. The
//! Host decodes an upload as its declared kind only and caches it as a
//! WebP; a read answers a `data:image/webp;base64,…` URL.

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::{CommandError, Done};
use rekindle_types::image_kind::ImageKind;
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

#[tauri::command]
pub async fn set_community_avatar(
    state: State<'_, SharedState>,
    community_id: String,
    bytes: Vec<u8>,
    kind: ImageKind,
) -> Result<Done<String>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::CommunitySetAvatar {
        community: community_id,
        image: bytes,
        kind,
    });
    host_call::call(&state, request).await
}

#[tauri::command]
pub async fn set_community_banner(
    state: State<'_, SharedState>,
    community_id: String,
    bytes: Vec<u8>,
    kind: ImageKind,
) -> Result<Done<String>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::CommunitySetBanner {
        community: community_id,
        image: bytes,
        kind,
    });
    host_call::call(&state, request).await
}

#[tauri::command]
pub async fn get_community_avatar_data_url(
    state: State<'_, SharedState>,
    community_id: String,
    hash: String,
) -> Result<Done<Option<String>>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::CommunityAvatarGet {
        community: community_id,
        hash,
    });
    host_call::call(&state, request).await
}

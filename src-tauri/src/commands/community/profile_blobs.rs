//! A community's avatar and banner pictures (plan E3.1 D4): Host calls. The
//! Host decodes an upload as its declared kind only and caches it as a
//! WebP; a read answers a `data:image/webp;base64,…` URL.

use rekindle_ipc::protocol::SettingsRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::{CommandError, Done};
use rekindle_types::image_kind::ImageKind;
use tauri::State;

use crate::state::SharedState;

host_command! {
    state;
    pub async fn set_community_avatar(
        state: State<'_, SharedState>,
        community_id: String,
        bytes: Vec<u8>,
        kind: ImageKind,
    ) -> Result<Done<String>, CommandError> {
        IpcRequest::Community(CommunityRequest::Settings(
            SettingsRequest::CommunitySetAvatar {
                community: community_id,
                image: bytes,
                kind,
            },
        ))
    }
}

host_command! {
    state;
    pub async fn set_community_banner(
        state: State<'_, SharedState>,
        community_id: String,
        bytes: Vec<u8>,
        kind: ImageKind,
    ) -> Result<Done<String>, CommandError> {
        IpcRequest::Community(CommunityRequest::Settings(
            SettingsRequest::CommunitySetBanner {
                community: community_id,
                image: bytes,
                kind,
            },
        ))
    }
}

host_command! {
    state;
    pub async fn get_community_avatar_data_url(
        state: State<'_, SharedState>,
        community_id: String,
        hash: String,
    ) -> Result<Done<Option<String>>, CommandError> {
        IpcRequest::Community(CommunityRequest::Settings(
            SettingsRequest::CommunityAvatarGet {
                community: community_id,
                hash,
            },
        ))
    }
}

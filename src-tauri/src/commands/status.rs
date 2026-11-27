//! Our status, message, name and avatar are Host calls (plan E1.1): the
//! Host owns our presence and profile and publishes every change.

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::presence::OwnPresenceView;
use rekindle_types::dto::{CommandError, Done};
use rekindle_types::image_kind::ImageKind;
use rekindle_types::presence::SessionStatus;
use tauri::State;

use crate::state::SharedState;

host_command! {
    state;
    /// Set our status (`IpcRequest::PresenceSet`); "Appear Offline" is
    /// `invisible`.
    pub async fn set_status(
        status: SessionStatus,
        state: State<'_, SharedState>,
    ) -> Result<Done<OwnPresenceView>, CommandError> {
        IpcRequest::PresenceSet { status }
    }
}

host_command! {
    state;
    /// Change the name we show (`IpcRequest::ProfileSetNickname`).
    pub async fn set_nickname(
        nickname: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<String>, CommandError> {
        IpcRequest::ProfileSetNickname {
            display_name: nickname,
        }
    }
}

host_command! {
    state;
    /// Our bio (`IpcRequest::ProfileBioGet`, plan E7.2); `None` when we have
    /// none.
    pub async fn get_bio(state: State<'_, SharedState>) -> Result<Done<Option<String>>, CommandError> {
        IpcRequest::ProfileBioGet
    }
}

host_command! {
    state;
    /// A friend's bio as they last sent it (`IpcRequest::FriendBioGet`, plan
    /// E7.2); `None` when they sent none.
    pub async fn get_friend_bio(
        public_key: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<Option<String>>, CommandError> {
        IpcRequest::FriendBioGet { public_key }
    }
}

host_command! {
    state;
    /// Change our bio (`IpcRequest::ProfileSetBio`, plan E7.2): blank removes
    /// it; the Host sends it to every accepted friend. The bio as stored.
    pub async fn set_bio(
        bio: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<Option<String>>, CommandError> {
        IpcRequest::ProfileSetBio { bio }
    }
}

host_command! {
    state;
    /// Change our avatar (`IpcRequest::ProfileSetAvatar`): `kind` is the file
    /// picker's MIME type; the Host decodes the picture as that kind only and
    /// makes it a 128 px WebP.
    pub async fn set_avatar(
        avatar_data: Vec<u8>,
        kind: ImageKind,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::ProfileSetAvatar {
            image: avatar_data,
            kind,
        }
    }
}

host_command! {
    state;
    /// Our own or a friend's avatar as WebP bytes (`IpcRequest::AvatarGet`).
    pub async fn get_avatar(
        public_key: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<Option<Vec<u8>>>, CommandError> {
        IpcRequest::AvatarGet { public_key }
    }
}

host_command! {
    state;
    /// Set our status message (`IpcRequest::PresenceSetMessage`).
    pub async fn set_status_message(
        message: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<OwnPresenceView>, CommandError> {
        IpcRequest::PresenceSetMessage { message }
    }
}

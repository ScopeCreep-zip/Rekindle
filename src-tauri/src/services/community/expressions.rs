//! The soundboard's upload and play, over the Host's channel adapter
//! (`HostChannel`). Their requests are plan E4.1's; until then these
//! commands call the Host directly.

use crate::state::SharedState;

fn host(state: &SharedState) -> Result<std::sync::Arc<rekindle_node::host::Host>, String> {
    state
        .host
        .current()
        .ok_or_else(|| "not logged in".to_string())
}

pub async fn upload_soundboard_sound(
    state: &SharedState,
    community_id: &str,
    name: &str,
    bytes: Vec<u8>,
    tags: Vec<String>,
    duration_seconds: f32,
    volume: f32,
    emoji: Option<String>,
) -> Result<String, String> {
    host(state)?
        .channel()
        .upload_soundboard_sound(rekindle_channel::UploadSoundboardSoundParams {
            community_id,
            name,
            bytes,
            tags,
            duration_seconds,
            volume,
            emoji,
        })
        .await
        .map_err(|e| e.to_string())
}

pub fn play_soundboard(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
    expression_id_hex: &str,
) -> Result<(), String> {
    host(state)?
        .channel()
        .play_soundboard(community_id, channel_id, expression_id_hex)
        .map_err(|e| e.to_string())
}

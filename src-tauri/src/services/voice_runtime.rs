//! Phase 23.C — voice-handler Tauri-runtime orchestration lifted from
//! `commands/voice.rs`. Hosts:
//! * `list_audio_devices_inner` — pure DTO mapping over
//!   `rekindle_voice::capture::enumerate_audio_devices`.
//! * `persist_audio_device_prefs` — write the user's selected devices to
//!   the `preferences.json` Tauri store.
//! * `apply_stage_audience_gate` — force-mute when joining a Stage channel
//!   as an audience member (not on the speaker list).
//!
//! The Tauri command handlers in `commands/voice.rs` stay as ≤15-LoC
//! delegations.

use tauri_plugin_store::StoreExt;

use crate::state::{ChannelType, SharedState};

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevices {
    pub input_devices: Vec<AudioDeviceInfo>,
    pub output_devices: Vec<AudioDeviceInfo>,
}

pub fn list_audio_devices_inner() -> AudioDevices {
    let devices = rekindle_voice::capture::enumerate_audio_devices();
    AudioDevices {
        input_devices: devices
            .input_devices
            .into_iter()
            .map(|(name, is_default)| AudioDeviceInfo {
                id: name.clone(),
                name,
                is_default,
            })
            .collect(),
        output_devices: devices
            .output_devices
            .into_iter()
            .map(|(name, is_default)| AudioDeviceInfo {
                id: name.clone(),
                name,
                is_default,
            })
            .collect(),
    }
}

pub fn persist_audio_device_prefs(
    app: &tauri::AppHandle,
    input_device: Option<&str>,
    output_device: Option<&str>,
) -> Result<(), String> {
    let store = app.store("preferences.json").map_err(|e| e.to_string())?;
    let mut prefs: crate::commands::settings::Preferences = store
        .get("preferences")
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    prefs.input_device = input_device.map(str::to_string);
    prefs.output_device = output_device.map(str::to_string);
    let val = serde_json::to_value(&prefs).map_err(|e| e.to_string())?;
    store.set("preferences", val);
    store.save().map_err(|e| e.to_string())
}

pub fn apply_stage_audience_gate(state: &SharedState, community_id: &str, channel_id: &str) {
    let (is_stage, my_pseudonym, speakers) = {
        let communities = state.communities.read();
        let Some(community) = communities.get(community_id) else {
            return;
        };
        let Some(channel) = community
            .channels
            .iter()
            .find(|channel| channel.id == channel_id)
        else {
            return;
        };
        (
            matches!(channel.channel_type, ChannelType::Stage),
            community.my_pseudonym_key.clone().unwrap_or_default(),
            channel.stage_speakers.clone(),
        )
    };

    if !is_stage || speakers.contains(&my_pseudonym) {
        return;
    }

    state.force_voice_mute();
}

pub fn build_voice_session_deps(
    app: &tauri::AppHandle,
    state: &SharedState,
) -> Result<std::sync::Arc<dyn rekindle_voice::VoiceSessionDeps>, String> {
    use tauri::Manager as _;
    let pool = app
        .try_state::<crate::db::DbPool>()
        .ok_or("DbPool not installed")?
        .inner()
        .clone();
    Ok(crate::services::voice_adapter::VoiceAdapter::new(
        state.clone(),
        app.clone(),
        pool,
    ))
}

pub fn build_voice_signaling_deps(
    app: &tauri::AppHandle,
    state: &SharedState,
) -> Result<std::sync::Arc<dyn rekindle_voice::signaling::VoiceSignalingDeps>, String> {
    use tauri::Manager as _;
    let pool = app
        .try_state::<crate::db::DbPool>()
        .ok_or("DbPool not installed")?
        .inner()
        .clone();
    Ok(
        crate::services::voice_signaling_adapter::VoiceSignalingAdapter::new(
            state.clone(),
            app.clone(),
            pool,
        ),
    )
}

pub async fn join_voice_channel_inner(
    channel_id: &str,
    community_id: Option<&str>,
    app: &tauri::AppHandle,
    state: &SharedState,
) -> Result<(), String> {
    if let Some(cid) = community_id {
        crate::commands::community::require_permission(
            state,
            cid,
            rekindle_protocol::dht::community::permissions_v2::Permissions::CONNECT,
        )?;
    }
    let deps = build_voice_session_deps(app, state)?;
    rekindle_voice::session::join_voice_channel(&deps, channel_id, community_id)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(cid) = community_id {
        apply_stage_audience_gate(state, cid, channel_id);
    }
    Ok(())
}

pub fn server_mute_member_inner(
    community_id: &str,
    channel_id: &str,
    target_pseudonym: &str,
    muted: bool,
    app: &tauri::AppHandle,
    state: &SharedState,
) -> Result<(), String> {
    crate::commands::community::require_permission(
        state,
        community_id,
        rekindle_protocol::dht::community::permissions_v2::Permissions::MUTE_MEMBERS,
    )?;
    let deps = build_voice_signaling_deps(app, state)?;
    rekindle_voice::signaling::server_mute_member(
        &deps,
        community_id,
        channel_id,
        target_pseudonym,
        muted,
    );
    Ok(())
}

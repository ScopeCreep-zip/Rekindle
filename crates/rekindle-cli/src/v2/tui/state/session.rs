//! Session state — persisted across TUI restarts.
//!
//! Serialized to `${XDG_STATE_HOME}/rekindle/tui_state.json` on quit,
//! restored on startup. Only navigation metadata is persisted — no user
//! content (draft text, message bodies, peer names) touches the filesystem.
//! The daemon's encrypted vault is the sole persistence layer for content.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::v2::tui::components::input_box::InputBox;
use super::channels::ChannelKey;

/// Persisted TUI session state — navigation metadata only.
/// No user content. No draft text. No message bodies.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PersistedSession {
    pub active_tab: Option<String>,
    pub active_community: Option<String>,
    pub active_channel: Option<String>,
    pub sidebar_visible: bool,
    pub dm_selected_peer: Option<String>,
    /// Timezone display preference. Defaults to Local if missing (backwards compat).
    #[serde(default)]
    pub timezone: crate::v2::helpers::TimezoneMode,
    /// Per-community theme overrides. Keyed by governance_key.
    #[serde(default)]
    pub community_themes: HashMap<String, String>,
    /// Per-DM theme overrides. Keyed by peer_key.
    #[serde(default)]
    pub dm_themes: HashMap<String, String>,
}

/// Pending session restore — validated after `CommunityListLoaded`.
#[derive(Clone, Debug)]
pub struct PendingRestore {
    pub community: String,
    pub channel: Option<String>,
}

/// Live session state. InputBox instances hold draft text in memory only —
/// lost on TUI exit. This is intentional: draft text is user content that
/// must not be written to an unencrypted filesystem.
#[derive(Debug)]
pub struct SessionState {
    /// Selected DM peer key (not index — stable across reordering).
    pub dm_selected_peer: Option<String>,
    /// Selected friend public key.
    pub friend_selected_key: Option<String>,
    /// Per-DM-thread InputBox instances, keyed by peer_key. Memory-only.
    pub dm_inputs: HashMap<String, InputBox>,
    /// Per-channel InputBox instances. Memory-only.
    pub channel_inputs: HashMap<ChannelKey, InputBox>,
    /// Per-split-DM InputBox instances, keyed by peer_key. Memory-only.
    pub split_dm_inputs: HashMap<String, InputBox>,
    /// Per-thread InputBox instances, keyed by thread_id. Memory-only.
    pub thread_inputs: HashMap<String, InputBox>,
    /// Deferred session restore — consumed after `CommunityListLoaded`.
    pub pending_restore: Option<PendingRestore>,
    /// Per-community theme override names. Keyed by governance_key.
    pub community_themes: HashMap<String, String>,
    /// Per-DM theme override names. Keyed by peer_key.
    pub dm_themes: HashMap<String, String>,
}

impl SessionState {
    pub fn new() -> Self {
        Self {
            dm_selected_peer: None,
            friend_selected_key: None,
            dm_inputs: HashMap::new(),
            channel_inputs: HashMap::new(),
            split_dm_inputs: HashMap::new(),
            thread_inputs: HashMap::new(),
            pending_restore: None,
            community_themes: HashMap::new(),
            dm_themes: HashMap::new(),
        }
    }

    /// Restore from persisted state. Only navigation metadata is restored.
    /// InputBox instances start empty — no draft text survives restart.
    pub fn restore(persisted: &PersistedSession) -> Self {
        let mut session = Self::new();
        session.dm_selected_peer = persisted.dm_selected_peer.clone();
        if let (Some(community), channel) = (
            persisted.active_community.clone(),
            persisted.active_channel.clone(),
        ) {
            session.pending_restore = Some(PendingRestore { community, channel });
        }
        session.community_themes = persisted.community_themes.clone();
        session.dm_themes = persisted.dm_themes.clone();
        session
    }

    /// Serialize to persisted form. Navigation metadata only.
    pub fn to_persisted(
        &self,
        active_tab: Option<&str>,
        active_community: Option<&str>,
        active_channel: Option<&str>,
        sidebar_visible: bool,
        timezone: crate::v2::helpers::TimezoneMode,
    ) -> PersistedSession {
        PersistedSession {
            active_tab: active_tab.map(str::to_string),
            active_community: active_community.map(str::to_string),
            active_channel: active_channel.map(str::to_string),
            sidebar_visible,
            dm_selected_peer: self.dm_selected_peer.clone(),
            timezone,
            community_themes: self.community_themes.clone(),
            dm_themes: self.dm_themes.clone(),
        }
    }
}

impl Default for SessionState {
    fn default() -> Self {
        Self::new()
    }
}

/// Path to the TUI session state file.
pub fn state_path() -> Option<PathBuf> {
    let state_dir = dirs::state_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join(".local/state")))?
        .join("rekindle");
    let _ = std::fs::create_dir_all(&state_dir);
    Some(state_dir.join("tui_state.json"))
}

/// Load saved session state. Returns default if the file is missing or malformed.
pub fn load() -> PersistedSession {
    let Some(path) = state_path() else {
        return PersistedSession::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(json) => serde_json::from_str(&json).unwrap_or_default(),
        Err(_) => PersistedSession::default(),
    }
}

/// Save session state. Best-effort — failures are logged, never block quit.
pub fn save(state: &PersistedSession) {
    let Some(path) = state_path() else {
        tracing::warn!("cannot determine TUI state path — session state not saved");
        return;
    };
    match serde_json::to_string_pretty(state) {
        Ok(json) => {
            let write_result = {
                #[cfg(unix)]
                {
                    use std::io::Write;
                    use std::os::unix::fs::OpenOptionsExt;
                    std::fs::OpenOptions::new()
                        .create(true)
                        .write(true)
                        .truncate(true)
                        .mode(0o600)
                        .open(&path)
                        .and_then(|mut f| f.write_all(json.as_bytes()))
                }
                #[cfg(not(unix))]
                {
                    std::fs::write(&path, &json)
                }
            };
            if let Err(e) = write_result {
                tracing::warn!(error = %e, "failed to save TUI session state");
            }
        }
        Err(e) => tracing::warn!(error = %e, "failed to serialize TUI session state"),
    }
}

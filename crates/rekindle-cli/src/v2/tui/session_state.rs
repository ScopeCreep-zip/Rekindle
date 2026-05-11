//! TUI session state — persisted across restarts.
//!
//! Saves minimal UI state to `${XDG_STATE_HOME}/rekindle/tui_state.json`
//! on quit and restores on startup. Only saves what the user would notice
//! is missing: which view was selected, sidebar visibility. Does NOT save
//! scroll positions, selection indices, or cached data — those are rebuilt
//! from the daemon on each startup.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Persisted TUI session state.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TuiSessionState {
    /// The last active tab ID ("dashboard", "communities", "dms", "friends").
    pub active_tab: Option<String>,
    /// The last active community governance key (for restoring community context).
    pub active_community: Option<String>,
    /// The last active channel within the community.
    pub active_channel: Option<String>,
    /// Whether the sidebar was visible.
    pub sidebar_visible: bool,
}

/// Path to the TUI session state file.
fn state_path() -> Option<PathBuf> {
    let state_dir = dirs::state_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join(".local/state")))?
        .join("rekindle");
    let _ = std::fs::create_dir_all(&state_dir);
    Some(state_dir.join("tui_state.json"))
}

/// Load saved session state. Returns default if the file doesn't exist
/// or is malformed (never fails — degraded state is acceptable).
pub fn load() -> TuiSessionState {
    let Some(path) = state_path() else {
        return TuiSessionState::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(json) => serde_json::from_str(&json).unwrap_or_default(),
        Err(_) => TuiSessionState::default(),
    }
}

/// Save session state. Best-effort — failures are logged but don't
/// block the quit flow.
pub fn save(state: &TuiSessionState) {
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

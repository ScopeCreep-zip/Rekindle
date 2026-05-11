//! XDG path resolution for session, config, and storage directories.

use std::path::{Path, PathBuf};
use anyhow::Context;

/// `${XDG_STATE_HOME}/rekindle/session.json`
pub fn session_path() -> anyhow::Result<PathBuf> {
    let state_dir = dirs::state_dir()
        .unwrap_or_else(|| {
            dirs::home_dir()
                .expect("home directory must exist")
                .join(".local/state")
        })
        .join("rekindle");
    std::fs::create_dir_all(&state_dir)
        .with_context(|| format!("failed to create state directory: {}", state_dir.display()))?;
    Ok(state_dir.join("session.json"))
}

/// `${XDG_CONFIG_HOME}/rekindle/`
pub fn config_dir() -> anyhow::Result<PathBuf> {
    let dir = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine config directory"))?
        .join("rekindle");
    Ok(dir)
}

/// `${XDG_DATA_HOME}/rekindle/veilid/`
pub fn storage_dir(override_path: Option<&Path>) -> anyhow::Result<PathBuf> {
    if let Some(p) = override_path {
        return Ok(p.to_path_buf());
    }
    let dir = dirs::data_dir()
        .unwrap_or_else(|| {
            dirs::home_dir()
                .expect("home directory must exist")
                .join(".local/share")
        })
        .join("rekindle/veilid");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create storage directory: {}", dir.display()))?;
    Ok(dir)
}

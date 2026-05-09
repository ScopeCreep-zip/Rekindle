//! Persistent state management.
//!
//! Bridges between the IPC layer (which knows nothing about Veilid) and the
//! rekindle-transport crate (which owns all Veilid operations). This module
//! manages:
//!
//! - Session loading/saving (session.json, atomic write)
//! - State directory creation and permissions
//!
//! **Boundary**: This module imports `rekindle_transport::Session` and
//! `rekindle_transport::TransportConfig`. All network operations are
//! delegated to rekindle-transport's broadcast/ and subscriptions/ modules.



pub mod audit;
pub mod keystore;
pub mod local_messages;
pub mod signal_sessions;

use std::path::{Path, PathBuf};

use rekindle_transport::Session;

/// State directory paths resolved from XDG conventions.
#[derive(Debug, Clone)]
pub struct StatePaths {
    /// Root state directory: `~/.local/state/rekindle/`
    pub state_dir: PathBuf,
    /// Session file: `~/.local/state/rekindle/session.json`
    pub session_file: PathBuf,
    /// Log directory: `~/.local/state/rekindle/logs/`
    pub log_dir: PathBuf,
    /// Veilid storage: `~/.local/share/rekindle/veilid/`
    pub veilid_dir: PathBuf,
    /// Config directory: `~/.config/rekindle/`
    pub config_dir: PathBuf,
}

impl StatePaths {
    /// Resolve all paths from XDG base directories.
    ///
    /// Uses `$XDG_STATE_HOME` (default `~/.local/state`) for state,
    /// `$XDG_DATA_HOME` (default `~/.local/share`) for Veilid storage,
    /// `$XDG_CONFIG_HOME` (default `~/.config`) for configuration.
    pub fn resolve() -> anyhow::Result<Self> {
        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .or_else(|_| dirs_fallback())
            .map_err(|()| anyhow::anyhow!("cannot determine home directory"))?;

        let state_home = std::env::var("XDG_STATE_HOME")
            .map_or_else(|_| home.join(".local/state"), PathBuf::from);

        let data_home = std::env::var("XDG_DATA_HOME")
            .map_or_else(|_| home.join(".local/share"), PathBuf::from);

        let config_home = std::env::var("XDG_CONFIG_HOME")
            .map_or_else(|_| home.join(".config"), PathBuf::from);

        let state_dir = state_home.join("rekindle");
        let session_file = state_dir.join("session.json");
        let log_dir = state_dir.join("logs");
        let veilid_dir = data_home.join("rekindle/veilid");
        let config_dir = config_home.join("rekindle");

        Ok(Self {
            state_dir,
            session_file,
            log_dir,
            veilid_dir,
            config_dir,
        })
    }

    /// Ensure all required directories exist with correct permissions.
    pub async fn ensure_directories(&self) -> anyhow::Result<()> {
        for dir in [&self.state_dir, &self.log_dir, &self.veilid_dir, &self.config_dir] {
            tokio::fs::create_dir_all(dir).await?;

            // [RC-6] State and veilid dirs: owner-only (0700).
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if dir == &self.state_dir || dir == &self.veilid_dir {
                    tokio::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).await?;
                }
            }
        }
        Ok(())
    }
}

/// Load session from disk if it exists.
pub fn load_session(path: &Path) -> anyhow::Result<Option<Session>> {
    match Session::load(path) {
        Ok(Some(session)) => Ok(Some(session)),
        Ok(None) => Ok(None),
        Err(e) => Err(anyhow::anyhow!("failed to load session: {e}")),
    }
}

/// Save session to disk via atomic write.
pub fn save_session(session: &Session, path: &Path) -> anyhow::Result<()> {
    session
        .save(path)
        .map_err(|e| anyhow::anyhow!("failed to save session: {e}"))
}

/// Fallback home directory resolution when $HOME is not set.
/// Uses the current UID to look up the home directory from the system
/// password database. Returns Err if resolution fails.
fn dirs_fallback() -> std::result::Result<PathBuf, ()> {
    // $HOME is the primary source. If it's set, use it.
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return Ok(PathBuf::from(home));
        }
    }
    // No $HOME — cannot determine home directory without platform-specific
    // getpwuid FFI which we avoid (#![forbid(unsafe_code)]). Fail closed.
    Err(())
}

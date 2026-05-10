//! Persistent state management — XDG directory resolution only.
//!
//! Secret storage, session persistence, audit logging, and key management
//! are in `rekindle-storage`. This module provides only the path resolution
//! for the daemon's state directories.

use std::path::PathBuf;

/// State directory paths resolved from XDG conventions.
#[derive(Debug, Clone)]
pub struct StatePaths {
    /// Root state directory: `~/.local/state/rekindle/`
    pub state_dir: PathBuf,
    /// Session file: `~/.local/state/rekindle/session.json`
    pub session_file: PathBuf,
    /// Vault database: `~/.local/state/rekindle/vault.db`
    pub vault_db: PathBuf,
    /// Log directory: `~/.local/state/rekindle/logs/`
    pub log_dir: PathBuf,
    /// Veilid storage: `~/.local/share/rekindle/veilid/`
    pub veilid_dir: PathBuf,
    /// Config directory: `~/.config/rekindle/`
    pub config_dir: PathBuf,
}

impl StatePaths {
    /// Resolve all paths from XDG base directories.
    pub fn resolve() -> anyhow::Result<Self> {
        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .map_err(|_| anyhow::anyhow!("cannot determine home directory"))?;

        let state_home = std::env::var("XDG_STATE_HOME")
            .map_or_else(|_| home.join(".local/state"), PathBuf::from);

        let data_home = std::env::var("XDG_DATA_HOME")
            .map_or_else(|_| home.join(".local/share"), PathBuf::from);

        let config_home = std::env::var("XDG_CONFIG_HOME")
            .map_or_else(|_| home.join(".config"), PathBuf::from);

        let state_dir = state_home.join("rekindle");

        Ok(Self {
            session_file: state_dir.join("session.json"),
            vault_db: state_dir.join("vault.db"),
            log_dir: state_dir.join("logs"),
            state_dir,
            veilid_dir: data_home.join("rekindle/veilid"),
            config_dir: config_home.join("rekindle"),
        })
    }

    /// Ensure all required directories exist with correct permissions.
    pub async fn ensure_directories(&self) -> anyhow::Result<()> {
        for dir in [&self.state_dir, &self.log_dir, &self.veilid_dir, &self.config_dir] {
            tokio::fs::create_dir_all(dir).await?;
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

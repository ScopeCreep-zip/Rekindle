//! `DataRoot`: the one set of directories every Rekindle host and frontend
//! uses (plan C5, `frontend-decoupling.md` F3). It is plain path
//! resolution, so it lives here where the thin frontends can reach it;
//! `rekindle_db::paths` re-exports it for the hosts.
//!
//! The desktop, `rekindled` and the CLI resolve their files here, so an
//! identity created by one is the identity the others see. The rules are
//! Tauri 2.12.1's app-directory rules (`tauri::path::PathResolver::
//! app_{data,config,log}_dir`), computed with the same `dirs` release
//! Tauri uses, so the desktop's folders do not move
//! (`evidence/c5-data-root-research.md`).

use std::path::PathBuf;

/// The bundle identifier the directories are named after. Equal to
/// `src-tauri/tauri.conf.json`'s `identifier` (tested).
pub const IDENTIFIER: &str = "com.rekindle.app";

/// A platform directory could not be resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("cannot resolve the platform {0} directory")]
pub struct PathsError(pub &'static str);

/// Where every Rekindle host keeps its files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataRoot {
    /// `dirs::data_dir()/com.rekindle.app`: Veilid storage, the file cache,
    /// and the node lock.
    pub data: PathBuf,
    /// `dirs::config_dir()/com.rekindle.app`: `config.toml`, and (until
    /// plan D1) the database and vault files. On macOS this is the same
    /// folder as `data`.
    pub config: PathBuf,
    /// Tauri's log directory: `~/Library/Logs/com.rekindle.app` on macOS,
    /// `dirs::data_local_dir()/com.rekindle.app/logs` elsewhere.
    pub logs: PathBuf,
    /// `data/state`: per-session state a reset removes (plan D1).
    pub state: PathBuf,
}

impl DataRoot {
    /// Resolve the directories for this user on this platform. Nothing is
    /// created.
    ///
    /// # Errors
    /// [`PathsError`] when the platform has no such directory (no home).
    pub fn resolve() -> Result<Self, PathsError> {
        let data = dirs::data_dir().ok_or(PathsError("data"))?.join(IDENTIFIER);
        let config = dirs::config_dir()
            .ok_or(PathsError("config"))?
            .join(IDENTIFIER);
        let logs = if cfg!(target_os = "macos") {
            dirs::home_dir()
                .ok_or(PathsError("home"))?
                .join("Library/Logs")
                .join(IDENTIFIER)
        } else {
            dirs::data_local_dir()
                .ok_or(PathsError("local data"))?
                .join(IDENTIFIER)
                .join("logs")
        };
        let state = data.join("state");
        Ok(Self {
            data,
            config,
            logs,
            state,
        })
    }

    /// Veilid's storage directory.
    #[must_use]
    pub fn veilid(&self) -> PathBuf {
        self.data.join("veilid")
    }

    /// The content-addressed file cache (per-community sub-directories).
    #[must_use]
    pub fn file_cache(&self) -> PathBuf {
        self.data.join("file_cache")
    }

    /// The SQLite database. In `config` until plan D1 moves it per identity.
    #[must_use]
    pub fn database(&self) -> PathBuf {
        self.config.join("rekindle.db")
    }

    /// Create every directory, owner-only (`0700`) on Unix: the root holds
    /// keys, the database and Veilid's node identity (RC-6).
    ///
    /// # Errors
    /// A directory cannot be created or its permissions set.
    pub fn create_dirs(&self) -> std::io::Result<()> {
        for dir in [
            &self.data,
            &self.config,
            &self.state,
            &self.logs,
            &self.veilid(),
        ] {
            std::fs::create_dir_all(dir)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
            }
        }
        Ok(())
    }

    /// The user layer of `config.toml`.
    #[must_use]
    pub fn user_config(&self) -> PathBuf {
        self.config.join("config.toml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The desktop's directories are named after `tauri.conf.json`'s
    /// identifier; the hosts that share them must use the same name.
    #[test]
    fn identifier_matches_tauri_conf() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../../../src-tauri/tauri.conf.json")).unwrap();
        assert_eq!(conf["identifier"], IDENTIFIER);
    }

    #[test]
    fn layout_follows_the_tauri_rules() {
        let root = DataRoot::resolve().unwrap();
        assert_eq!(root.data, dirs::data_dir().unwrap().join(IDENTIFIER));
        assert_eq!(root.config, dirs::config_dir().unwrap().join(IDENTIFIER));
        assert_eq!(root.state, root.data.join("state"));
        assert!(root.logs.ends_with(if cfg!(target_os = "macos") {
            "Library/Logs/com.rekindle.app"
        } else {
            "com.rekindle.app/logs"
        }));
    }
}

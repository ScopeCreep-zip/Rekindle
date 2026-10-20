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

/// The one child of [`DataRoot::file_cache`] still written: the desktop's
/// community avatars (`community_avatars/<community id>/`), until plan
/// E3.1 moves them under [`DataRoot::identity_files`]. Every other child is
/// a chunk directory of the layout before E3.0a, removed at start
/// (`rekindle_db::reset::prune_file_cache`).
pub const COMMUNITY_AVATARS: &str = "community_avatars";

/// A platform directory could not be resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("cannot resolve the platform {0} directory")]
pub struct PathsError(pub &'static str);

/// Where every Rekindle host keeps its files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataRoot {
    /// `dirs::data_dir()/com.rekindle.app`: `state/`, the node lock and,
    /// until plan E3.3, the desktop's old `file_cache`.
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

    /// Veilid's storage directory, `state/veilid` (plan D1): it resets with
    /// the identities.
    #[must_use]
    pub fn veilid(&self) -> PathBuf {
        self.state.join("veilid")
    }

    /// Create Veilid's storage directory, owner-only, and return it. Called
    /// when the node starts, after the layout check, so a fresh `state/`
    /// is still empty when the check reads it.
    ///
    /// # Errors
    /// The directory cannot be created or its permissions set.
    pub fn create_veilid_dir(&self) -> std::io::Result<PathBuf> {
        let dir = self.veilid();
        create_private_dir(&dir)?;
        Ok(dir)
    }

    /// The desktop's old shared cache, `data/file_cache`. Before plan E3.0a
    /// it held one chunk directory per community id; now only
    /// [`COMMUNITY_AVATARS`] is written there. The desktop prunes every
    /// other child at start, the layout check removes it whole on a reset,
    /// and E3.3 deletes it with its last caller's port onto
    /// [`DataRoot::identity_files`].
    #[must_use]
    pub fn file_cache(&self) -> PathBuf {
        self.data.join("file_cache")
    }

    /// Create every directory, owner-only (`0700`) on Unix: the root holds
    /// keys, the databases and Veilid's node identity (RC-6). Veilid's own
    /// directory is created when the node starts (`create_veilid_dir`).
    ///
    /// # Errors
    /// A directory cannot be created or its permissions set.
    pub fn create_dirs(&self) -> std::io::Result<()> {
        for dir in [&self.data, &self.config, &self.state, &self.logs] {
            create_private_dir(dir)?;
        }
        Ok(())
    }

    /// An identity's database, `state/{pk}.db` (plan D1).
    #[must_use]
    pub fn identity_db(&self, public_key: &str) -> PathBuf {
        self.state.join(format!("{public_key}.db"))
    }

    /// An identity's vault, `state/{pk}.vault` (its header is
    /// `state/{pk}.vault.hdr`, plan D1).
    #[must_use]
    pub fn identity_vault(&self, public_key: &str) -> PathBuf {
        self.state.join(format!("{public_key}.vault"))
    }

    /// An identity's file cache, `state/{pk}.files/` (plan E3.0a): every
    /// directory below it has a keyed-hash name
    /// (`rekindle_files::cache::scope_root`), so before unlock it shows only
    /// sizes and counts. Identity delete removes it with `{pk}.db`.
    #[must_use]
    pub fn identity_files(&self, public_key: &str) -> PathBuf {
        self.state.join(format!("{public_key}.files"))
    }

    /// The identity registry, `state/identities.json` (plan D1).
    #[must_use]
    pub fn registry(&self) -> PathBuf {
        self.state.join("identities.json")
    }

    /// The layout version, `state/layout-version` (plan D1): the decimal
    /// schema version the root was laid out for.
    #[must_use]
    pub fn layout_version(&self) -> PathBuf {
        self.state.join("layout-version")
    }

    /// The user layer of `config.toml`.
    #[must_use]
    pub fn user_config(&self) -> PathBuf {
        self.config.join("config.toml")
    }
}

/// Create `dir` and set it owner-only (`0700`) on Unix.
fn create_private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
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
        assert_eq!(root.identity_files("pk"), root.state.join("pk.files"));
        assert!(root.logs.ends_with(if cfg!(target_os = "macos") {
            "Library/Logs/com.rekindle.app"
        } else {
            "com.rekindle.app/logs"
        }));
    }
}

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

pub mod backup;

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
    /// `dirs::data_dir()/com.rekindle.app`: `state/` and the node lock (and
    /// the retired `file_cache` of an older install until it is removed).
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
    /// Where the GUI's webview keeps its stores (plan E7.1, quick wipe):
    /// the windows are built for it (`windows::base_builder`) and a wipe
    /// removes its [`WebviewStore::dirs`], so both follow this root.
    pub webview: WebviewStore,
    /// Where other apps on this machine keep their files (Steam's library,
    /// plan E7.15 IV3), decided with the root.
    pub apps: AppRoots,
}

/// The per-OS roots other apps keep their files under, resolved with the
/// [`DataRoot`] so the Host reads them from its own value, never from the
/// process environment: a root under one base gets roots under that base
/// too, and reads nothing of the user's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppRoots {
    /// The user's home directory (Steam on macOS and Linux).
    pub home: PathBuf,
    /// `%ProgramFiles(x86)%` on Windows (Steam's default install); `None`
    /// elsewhere and under a base.
    pub program_files_x86: Option<PathBuf>,
    /// The file system root the OS's own apps sit under (`/`): a native
    /// game's fixed path in the game database (`/System/Applications/
    /// Chess.app`, `/usr/games/gnome-chess`, plan E7.15 CH1) is read under
    /// it. `base/system` under a base, so a contained Host sees none.
    pub system: PathBuf,
}

/// Where the GUI's webview keeps its stores, decided with the root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebviewStore {
    /// A real install ([`DataRoot::resolve`]): the webview's platform
    /// defaults, outside `DataRoot.data` for macOS WebKit
    /// (`~/Library/WebKit/<id>`, `~/Library/Caches/<id>`), Linux
    /// (`dirs::cache_dir()/<id>`; WebKitGTK's data directory is Tauri's
    /// `app_local_data_dir`, `DataRoot.data` itself) and Windows WebView2
    /// (`%LOCALAPPDATA%\<id>\EBWebView`).
    Platform(Vec<PathBuf>),
    /// A root under one base ([`DataRoot::under`], the WebDriver walk):
    /// the windows get this data directory (WebKitGTK and WebView2 keep
    /// every store in it) and, on macOS, where WKWebView has no directory
    /// API, a non-persistent store; nothing outside the base is written or
    /// wiped.
    Contained(PathBuf),
}

impl WebviewStore {
    /// The directories a wipe removes.
    #[must_use]
    pub fn dirs(&self) -> Vec<PathBuf> {
        match self {
            Self::Platform(dirs) => dirs.clone(),
            Self::Contained(dir) => vec![dir.clone()],
        }
    }
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
            webview: WebviewStore::Platform(platform_webview_dirs()?),
            apps: AppRoots {
                home: dirs::home_dir().ok_or(PathsError("home"))?,
                program_files_x86: std::env::var_os("ProgramFiles(x86)").map(PathBuf::from),
                system: PathBuf::from("/"),
            },
        })
    }

    /// The same layout under one base directory instead of the platform's:
    /// `base/data`, `base/config`, `base/logs`, `base/data/state`, and other
    /// apps' home `base/home`. Nothing is
    /// created. The WebDriver walk (plan E7.13) runs the desktop on such a
    /// root so it never opens the user's.
    #[must_use]
    pub fn under(base: &std::path::Path) -> Self {
        let data = base.join("data");
        Self {
            config: base.join("config"),
            logs: base.join("logs"),
            state: data.join("state"),
            data,
            webview: WebviewStore::Contained(base.join("webview")),
            apps: AppRoots {
                home: base.join("home"),
                program_files_x86: None,
                system: base.join("system"),
            },
        }
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

    /// The desktop's retired shared cache, `data/file_cache`. Nothing
    /// writes it since plan E3.1 (the pictures and chunks live under
    /// [`DataRoot::identity_files`]); the desktop removes it at start
    /// (`rekindle_db::reset::prune_file_cache`) and a reset removes it too.
    #[must_use]
    pub fn file_cache(&self) -> PathBuf {
        self.data.join("file_cache")
    }

    /// The device login skin, `data/skins/` (plan E7.7.2): `login.json`
    /// and at most one stripped, render-only package, the only skin data
    /// readable before unlock. It sits outside `state/`, so a schema reset
    /// keeps it.
    #[must_use]
    pub fn skins(&self) -> PathBuf {
        self.data.join("skins")
    }

    /// Create [`DataRoot::skins`], owner-only, and return it. Created on
    /// the first write of the login skin, never at start.
    ///
    /// # Errors
    /// The directory cannot be created or its permissions set.
    pub fn create_skins_dir(&self) -> std::io::Result<PathBuf> {
        let dir = self.skins();
        create_private_dir(&dir)?;
        Ok(dir)
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

    /// An identity's device vault on this device, `state/{pk}.device.vault`
    /// (plan E3.2 S7d-keys.7, ADR 0022 rule 4): the per-device secrets
    /// (governance device keys, the prekey record's owner, card HPKE
    /// secrets) that identity export never reads. Its random key lives in
    /// the identity vault (`VaultKey::DeviceVaultSecret`), so it has no header.
    #[must_use]
    pub fn device_vault(&self, public_key: &str) -> PathBuf {
        self.state.join(format!("{public_key}.device.vault"))
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

/// The webview's platform stores outside `DataRoot.data` (see
/// [`WebviewStore::Platform`]).
fn platform_webview_dirs() -> Result<Vec<PathBuf>, PathsError> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or(PathsError("home"))?;
        Ok(vec![
            home.join("Library/WebKit").join(IDENTIFIER),
            home.join("Library/Caches").join(IDENTIFIER),
        ])
    }
    #[cfg(windows)]
    {
        let local = dirs::data_local_dir().ok_or(PathsError("local data"))?;
        Ok(vec![local.join(IDENTIFIER).join("EBWebView")])
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let cache = dirs::cache_dir().ok_or(PathsError("cache"))?;
        Ok(vec![cache.join(IDENTIFIER)])
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
    #[test]
    fn under_lays_every_directory_out_below_the_base() {
        let base = std::path::Path::new("/tmp/walk-root");
        let root = super::DataRoot::under(base);
        for dir in [&root.data, &root.config, &root.logs, &root.state] {
            assert!(dir.starts_with(base), "{}", dir.display());
        }
        assert_eq!(root.state, base.join("data/state"));
        assert_ne!(root.data, root.config);
    }

    /// A wipe on a root under a base (the walk's) removes only webview
    /// stores below that base, never a real install's (Linux finding F1).
    #[test]
    fn a_contained_root_wipes_only_webview_stores_below_its_base() {
        let base = std::path::Path::new("/tmp/walk-root");
        let dirs = super::DataRoot::under(base).webview.dirs();
        assert!(!dirs.is_empty());
        for dir in &dirs {
            assert!(dir.starts_with(base), "{}", dir.display());
        }
    }

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
        assert!(matches!(root.webview, WebviewStore::Platform(_)));
        assert!(root.logs.ends_with(if cfg!(target_os = "macos") {
            "Library/Logs/com.rekindle.app"
        } else {
            "com.rekindle.app/logs"
        }));
    }
}

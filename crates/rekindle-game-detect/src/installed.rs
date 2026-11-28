//! The games we can launch (plan E7.15 IV3, Xfire's "N Installed Games"):
//! the bundled database's games that Steam has installed and the OS's own
//! native games present (plan E7.15 CH1), with the games we have played,
//! minus the hide list.
//!
//! Steam lists its libraries in `steamapps/libraryfolders.vdf` (each a
//! `"path"` value) and each installed game as
//! `<library>/steamapps/appmanifest_<appid>.acf` (an `"appid"` value), both
//! Valve `KeyValues` text. Only these local files are read; nothing leaves
//! the device (threat model I10), and the Host reads them only while game
//! detection is on.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rekindle_utils::paths::AppRoots;

use crate::database::GameDatabase;

/// Where Steam keeps its root on each OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteamLayout {
    MacOs,
    Linux,
    Windows,
}

impl SteamLayout {
    /// This build's OS.
    #[cfg(target_os = "macos")]
    pub const CURRENT: Self = Self::MacOs;
    /// This build's OS.
    #[cfg(windows)]
    pub const CURRENT: Self = Self::Windows;
    /// This build's OS.
    #[cfg(not(any(target_os = "macos", windows)))]
    pub const CURRENT: Self = Self::Linux;

    /// The Steam roots to try: under `home` on macOS and Linux (native and
    /// Flatpak installs), under `program_files_x86` on Windows.
    #[must_use]
    pub fn roots(self, home: &Path, program_files_x86: Option<&Path>) -> Vec<PathBuf> {
        match self {
            Self::MacOs => vec![home.join("Library/Application Support/Steam")],
            Self::Linux => vec![
                home.join(".steam/steam"),
                home.join(".local/share/Steam"),
                home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
            ],
            Self::Windows => program_files_x86
                .map(|dir| vec![dir.join("Steam")])
                .unwrap_or_default(),
        }
    }
}

/// The quoted strings of a `KeyValues` text, in order, unescaped (`\\`,
/// `\"`); braces and comments are skipped.
fn strings(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut value = String::new();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => {
                    if let Some(escaped) = chars.next() {
                        value.push(escaped);
                    }
                }
                other => value.push(other),
            }
        }
        out.push(value);
    }
    out
}

/// The values of every `key` in a `KeyValues` text (case-insensitive key).
fn values_of(text: &str, key: &str) -> Vec<String> {
    let tokens = strings(text);
    tokens
        .windows(2)
        .filter(|pair| pair[0].eq_ignore_ascii_case(key))
        .map(|pair| pair[1].clone())
        .collect()
}

/// The library paths `libraryfolders.vdf` lists.
#[must_use]
pub fn library_folders(vdf: &str) -> Vec<PathBuf> {
    values_of(vdf, "path")
        .into_iter()
        .map(PathBuf::from)
        .collect()
}

/// The application id an `appmanifest_<appid>.acf` names.
#[must_use]
pub fn manifest_app_id(acf: &str) -> Option<u32> {
    values_of(acf, "appid").first()?.parse().ok()
}

/// The Steam application ids installed under `roots`: each root's own
/// library and every library its `libraryfolders.vdf` lists. A missing or
/// unreadable file contributes nothing.
#[must_use]
pub fn steam_app_ids(roots: &[PathBuf]) -> BTreeSet<u32> {
    let mut libraries = BTreeSet::new();
    for root in roots {
        libraries.insert(root.clone());
        let vdf = root.join("steamapps").join("libraryfolders.vdf");
        if let Ok(text) = std::fs::read_to_string(vdf) {
            libraries.extend(library_folders(&text));
        }
    }
    let mut ids = BTreeSet::new();
    for library in libraries {
        let Ok(dir) = std::fs::read_dir(library.join("steamapps")) else {
            continue;
        };
        for entry in dir.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !(name.starts_with("appmanifest_") && name.ends_with(".acf")) {
                continue;
            }
            if let Some(id) = std::fs::read_to_string(entry.path())
                .ok()
                .as_deref()
                .and_then(manifest_app_id)
            {
                ids.insert(id);
            }
        }
    }
    ids
}

/// The Steam application ids installed under `apps`, this OS's Steam
/// roots in the Host's resolved [`AppRoots`] (never the process's own
/// home, so a Host on a contained root reads only that root).
#[must_use]
pub fn scan_steam(apps: &AppRoots) -> BTreeSet<u32> {
    steam_app_ids(&SteamLayout::CURRENT.roots(&apps.home, apps.program_files_x86.as_deref()))
}

/// The games in `db` whose native copy for `layout`'s OS is present under
/// `apps` (plan E7.15 CH1: the OS's own Chess), read from fixed database
/// paths only.
#[must_use]
pub fn native_installed(db: &GameDatabase, layout: SteamLayout, apps: &AppRoots) -> BTreeSet<u32> {
    db.entries()
        .filter(|entry| crate::launcher::native_present(entry, layout, apps))
        .map(|entry| entry.id)
        .collect()
}

/// The game ids we can launch: the games in `db` whose Steam id is in
/// `steam_app_ids`, the `native` games present, with `played`, minus
/// `hidden`; ascending.
#[must_use]
pub fn installed_games(
    db: &GameDatabase,
    steam_app_ids: &BTreeSet<u32>,
    native: &BTreeSet<u32>,
    played: &[u32],
    hidden: &[u32],
) -> Vec<u32> {
    steam_app_ids
        .iter()
        .flat_map(|app_id| db.ids_with_steam_app_id(*app_id))
        .chain(native.iter().copied())
        .chain(played.iter().copied())
        .filter(|id| !hidden.contains(id))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
#[path = "installed_tests.rs"]
mod tests;

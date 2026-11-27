//! The device login skin (plan E7.7.2 "Storage"): `data/skins/login.json`
//! `{skin: "builtin:<id>" | "dev:<hash>"}` and at most one stripped,
//! render-only package `data/skins/<hash>.rkskin`
//! (`rekindle_skin::login::strip`). The only skin data readable before
//! unlock; it sits outside `state/`, so a schema reset keeps it. With no
//! `login.json` the login skin is the built-in default.

use std::io::Write as _;
use std::path::Path;

use rekindle_db::paths::DataRoot;
use rekindle_skin::builtin;
use rekindle_skin::output::Canonical;
use serde::{Deserialize, Serialize};

use super::ids::{self, DEV};

/// The file's one field.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginFile {
    skin: String,
}

/// The extension of the stripped copy.
const EXT: &str = "rkskin";
const LOGIN_FILE: &str = "login.json";

/// A login skin that cannot be read or written.
#[derive(Debug, thiserror::Error)]
pub(crate) enum LoginSkinError {
    #[error("login skin I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("login.json is not {{\"skin\": <id>}}: {0}")]
    Malformed(#[from] serde_json::Error),
    #[error("login.json names {0:?}, which is no built-in and no stripped copy here")]
    Unknown(String),
}

/// The device login skin's id.
///
/// # Errors
/// `login.json` cannot be read, does not parse, or names a built-in that
/// does not exist or a copy that is not on disk.
pub(crate) fn read(root: &DataRoot) -> Result<String, LoginSkinError> {
    let path = root.skins().join(LOGIN_FILE);
    let text = match std::fs::read(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(builtin::DEFAULT.to_owned());
        }
        Err(e) => return Err(e.into()),
    };
    let file: LoginFile = serde_json::from_slice(&text)?;
    let known = match file.skin.strip_prefix(DEV) {
        Some(hash) => ids::is_hash(hash) && root.skins().join(copy_name(hash)).is_file(),
        None => rekindle_skin::conform::is_builtin(&file.skin),
    };
    if known {
        Ok(file.skin)
    } else {
        Err(LoginSkinError::Unknown(file.skin))
    }
}

/// Make the built-in `skin_id` the login skin; any stripped copy goes.
///
/// # Errors
/// A write or removal failed.
pub(crate) fn set_builtin(root: &DataRoot, skin_id: &str) -> Result<String, LoginSkinError> {
    let dir = root.create_skins_dir()?;
    write_file(&dir, skin_id)?;
    prune(&dir, None)?;
    Ok(skin_id.to_owned())
}

/// Make the stripped copy `copy` the login skin, replacing any other copy.
///
/// # Errors
/// A write or removal failed.
pub(crate) fn set_copy(root: &DataRoot, copy: &Canonical) -> Result<String, LoginSkinError> {
    let dir = root.create_skins_dir()?;
    let hash = copy.address();
    let name = copy_name(&hash);
    atomic_write(&dir.join(&name), &copy.bytes)?;
    let id = format!("{DEV}{hash}");
    write_file(&dir, &id)?;
    prune(&dir, Some(&name))?;
    Ok(id)
}

fn copy_name(hash: &str) -> String {
    format!("{hash}.{EXT}")
}

fn write_file(dir: &Path, skin: &str) -> Result<(), LoginSkinError> {
    let json = serde_json::to_vec(&LoginFile {
        skin: skin.to_owned(),
    })?;
    atomic_write(&dir.join(LOGIN_FILE), &json)?;
    Ok(())
}

/// Remove every stripped copy but `keep`.
fn prune(dir: &Path, keep: Option<&str>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let is_copy = Path::new(name).extension().and_then(|e| e.to_str()) == Some(EXT);
        if is_copy && Some(name) != keep {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// A 0600 temp file, synced, renamed over (`rekindle_db::registry::save`).
fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
#[path = "login_tests.rs"]
mod tests;

//! The data root's layout check and reset (plan D1, decision D9).
//!
//! A per-identity SQLCipher database cannot be read before its unlock, so
//! the reset trigger is a plaintext `state/layout-version` file holding the
//! decimal [`SCHEMA_VERSION`] the root was laid out for. Both hosts call
//! [`apply`] right after resolving and creating the root, before any store
//! opens:
//!
//! - [`LayoutCheck::Fresh`] (an empty `state/` and no old layout): the file
//!   is written.
//! - [`LayoutCheck::Current`]: nothing to do.
//! - [`LayoutCheck::Mismatch`] (another version, a non-empty `state/`
//!   without the file, or files of the layout before D1): [`run`] sweeps
//!   `state/`, the old layout and the retired directories are removed, and
//!   the file is written.
//!
//! Everything Rekindle owns per identity lives under `state/`, its file
//! cache `state/{pk}.files/` included (plan E3.0a), so the sweep is one
//! directory; the node lock, logs, webview data and the user's
//! `config.toml` live outside it and survive.

use std::path::{Path, PathBuf};

use crate::paths::DataRoot;
use crate::SCHEMA_VERSION;

/// What the root's layout needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutCheck {
    Current,
    Fresh,
    Mismatch,
}

/// A directory to sweep, and the entry names in it that survive.
#[derive(Debug, Clone)]
pub struct ResetDir<'a> {
    pub path: PathBuf,
    pub keep: &'a [&'a str],
}

/// The layout check or reset failed: a startup error the host shows.
#[derive(Debug, thiserror::Error)]
pub enum ResetError {
    #[error("data root reset, {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> ResetError + '_ {
    move |source| ResetError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Files of the layout before D1, outside `state/`: the shared database and
/// its WAL, the per-identity vaults and their sidecars in `config/`, and
/// Veilid's storage in `data/`.
fn old_layout(root: &DataRoot) -> Vec<PathBuf> {
    let mut out = vec![root.data.join("veilid")];
    if let Ok(entries) = std::fs::read_dir(&root.config) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("rekindle.db") || name.contains(".vault") {
                out.push(entry.path());
            }
        }
    }
    out.retain(|p| p.exists());
    out
}

/// Directories retired after D1 that a reset removes but that do not
/// trigger one: `data/file_cache/`, the desktop's old shared cache (plan
/// E3.0a moved the caches to `state/{pk}.files/` under keyed names, and
/// E3.1 the community pictures). It is no trigger because a reset would
/// drop every identity of an install that only still has that directory;
/// [`prune_file_cache`] removes it at start instead.
fn retired(root: &DataRoot) -> Vec<PathBuf> {
    vec![root.data.join("file_cache")]
}

/// Remove the retired `data/file_cache/`. Nothing writes it since plan
/// E3.1, but an install from before keeps its chunk directories and
/// community pictures there, under names that are community ids and
/// readable before unlock (threat model, A7). The desktop calls it at start
/// under the root's claim.
///
/// # Errors
/// The directory cannot be removed.
pub fn prune_file_cache(root: &DataRoot) -> Result<(), ResetError> {
    remove(&root.file_cache())
}

fn is_empty_dir(path: &Path) -> Result<bool, ResetError> {
    match std::fs::read_dir(path) {
        Ok(mut entries) => Ok(entries.next().is_none()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(io(path)(e)),
    }
}

/// What `root` needs.
///
/// # Errors
/// The state directory or the version file cannot be read.
pub fn check_layout(root: &DataRoot) -> Result<LayoutCheck, ResetError> {
    let file = root.layout_version();
    match std::fs::read_to_string(&file) {
        Ok(text) => Ok(match text.trim().parse::<i64>() {
            Ok(v) if v == SCHEMA_VERSION && old_layout(root).is_empty() => LayoutCheck::Current,
            _ => LayoutCheck::Mismatch,
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if is_empty_dir(&root.state)? && old_layout(root).is_empty() {
                Ok(LayoutCheck::Fresh)
            } else {
                Ok(LayoutCheck::Mismatch)
            }
        }
        Err(e) => Err(io(&file)(e)),
    }
}

/// Remove every entry of each directory except its `keep` names.
///
/// # Errors
/// The first entry that cannot be removed.
pub fn run(dirs: &[ResetDir<'_>]) -> Result<(), ResetError> {
    for dir in dirs {
        let entries = match std::fs::read_dir(&dir.path) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(io(&dir.path)(e)),
        };
        for entry in entries {
            let entry = entry.map_err(io(&dir.path))?;
            if dir.keep.iter().any(|k| entry.file_name() == **k) {
                continue;
            }
            remove(&entry.path())?;
        }
    }
    Ok(())
}

fn remove(path: &Path) -> Result<(), ResetError> {
    let result = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    match result {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io(path)(e)),
    }
}

fn write_version(root: &DataRoot) -> Result<(), ResetError> {
    std::fs::create_dir_all(&root.state).map_err(io(&root.state))?;
    let file = root.layout_version();
    std::fs::write(&file, SCHEMA_VERSION.to_string()).map_err(io(&file))
}

/// Check `root` and bring it to the current layout; returns what it found.
///
/// # Errors
/// A file that cannot be read, removed or written.
pub fn apply(root: &DataRoot) -> Result<LayoutCheck, ResetError> {
    let check = check_layout(root)?;
    match check {
        LayoutCheck::Current => {}
        LayoutCheck::Fresh => write_version(root)?,
        LayoutCheck::Mismatch => {
            tracing::warn!(
                root = %root.state.display(),
                version = SCHEMA_VERSION,
                "data root laid out for another version: resetting every identity"
            );
            run(&[ResetDir {
                path: root.state.clone(),
                keep: &[],
            }])?;
            for old in old_layout(root).into_iter().chain(retired(root)) {
                remove(&old)?;
            }
            write_version(root)?;
        }
    }
    Ok(check)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(dir: &Path) -> DataRoot {
        let data = dir.join("data");
        DataRoot {
            config: dir.join("config"),
            logs: dir.join("logs"),
            state: data.join("state"),
            data,
        }
    }

    #[test]
    fn a_fresh_root_is_stamped_then_current() {
        let dir = tempfile::tempdir().unwrap();
        let root = root(dir.path());
        assert_eq!(apply(&root).unwrap(), LayoutCheck::Fresh);
        assert_eq!(
            std::fs::read_to_string(root.layout_version()).unwrap(),
            SCHEMA_VERSION.to_string()
        );
        assert_eq!(apply(&root).unwrap(), LayoutCheck::Current);
    }

    #[test]
    fn a_root_for_another_version_is_swept_and_the_rest_survives() {
        let dir = tempfile::tempdir().unwrap();
        let root = root(dir.path());
        std::fs::create_dir_all(root.state.join("veilid")).unwrap();
        std::fs::create_dir_all(&root.config).unwrap();
        std::fs::write(root.identity_db("pk"), b"db").unwrap();
        std::fs::write(root.layout_version(), "1").unwrap();
        std::fs::write(root.config.join("config.toml"), b"user").unwrap();
        std::fs::write(root.data.join("rekindle.lock"), b"").unwrap();
        assert_eq!(apply(&root).unwrap(), LayoutCheck::Mismatch);
        assert!(!root.identity_db("pk").exists());
        assert!(!root.state.join("veilid").exists());
        assert!(
            root.config.join("config.toml").exists(),
            "user config survives"
        );
        assert!(
            root.data.join("rekindle.lock").exists(),
            "outside state survives"
        );
        assert_eq!(apply(&root).unwrap(), LayoutCheck::Current);
    }

    #[test]
    fn the_layout_before_d1_is_removed_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = root(dir.path());
        std::fs::create_dir_all(root.data.join("veilid")).unwrap();
        std::fs::create_dir_all(&root.config).unwrap();
        for f in [
            "rekindle.db",
            "rekindle.db-wal",
            "pk.vault",
            "pk.vault.salt",
            "config.toml",
        ] {
            std::fs::write(root.config.join(f), b"x").unwrap();
        }
        assert_eq!(check_layout(&root).unwrap(), LayoutCheck::Mismatch);
        apply(&root).unwrap();
        assert!(!root.data.join("veilid").exists());
        for f in [
            "rekindle.db",
            "rekindle.db-wal",
            "pk.vault",
            "pk.vault.salt",
        ] {
            assert!(!root.config.join(f).exists(), "{f}");
        }
        assert!(root.config.join("config.toml").exists());
    }

    #[test]
    fn a_reset_removes_the_old_file_cache_which_is_no_trigger_itself() {
        let dir = tempfile::tempdir().unwrap();
        let root = root(dir.path());
        let old_cache = root.data.join("file_cache").join("community-id");
        assert_eq!(apply(&root).unwrap(), LayoutCheck::Fresh);
        std::fs::create_dir_all(&old_cache).unwrap();
        assert_eq!(
            apply(&root).unwrap(),
            LayoutCheck::Current,
            "the retired cache alone does not reset the identities"
        );
        std::fs::create_dir_all(root.identity_files("pk")).unwrap();
        std::fs::write(root.layout_version(), "1").unwrap();
        assert_eq!(apply(&root).unwrap(), LayoutCheck::Mismatch);
        assert!(!root.data.join("file_cache").exists());
        assert!(!root.identity_files("pk").exists());
    }

    #[test]
    fn the_file_cache_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let root = root(dir.path());
        prune_file_cache(&root).unwrap();
        assert!(!root.file_cache().exists(), "a missing cache is no error");

        let avatar = root
            .file_cache()
            .join("community_avatars")
            .join("community-id")
            .join("hash.webp");
        std::fs::create_dir_all(avatar.parent().unwrap()).unwrap();
        std::fs::write(&avatar, b"webp").unwrap();
        let old_chunks = root.file_cache().join("community-id").join("attachment");
        std::fs::create_dir_all(&old_chunks).unwrap();
        std::fs::write(old_chunks.join("0.bin"), b"chunk").unwrap();

        prune_file_cache(&root).unwrap();
        assert!(!root.file_cache().exists());
    }

    #[test]
    fn run_keeps_the_allowlist() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("keep.me"), b"").unwrap();
        std::fs::write(dir.path().join("drop.me"), b"").unwrap();
        run(&[ResetDir {
            path: dir.path().to_path_buf(),
            keep: &["keep.me"],
        }])
        .unwrap();
        assert!(dir.path().join("keep.me").exists());
        assert!(!dir.path().join("drop.me").exists());
    }
}

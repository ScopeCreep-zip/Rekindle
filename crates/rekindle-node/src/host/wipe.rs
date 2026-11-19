//! The quick wipe (plan E7.1): everything that says Rekindle held an
//! identity on this device goes.
//!
//! The wipe set is the whole data root: `DataRoot.data` (`state/` with
//! every identity's database, vault, header and file cache, the registry
//! and Veilid's storage; `skins/`; `file_cache/`; the node lock),
//! `DataRoot.config` (`config.toml`, `agent-keys/`), `DataRoot.logs`, and the
//! bus runtime directory's `bus.pub`. The GUI also clears its webview
//! stores (`DataRoot::webview_dirs`), which only it holds.
//!
//! It runs in two parts because the node still holds Veilid's storage and
//! the root's node lock while the request is answered:
//! [`sweep_root`] takes everything else at once, and [`finish`] takes the
//! rest once the process's node has stopped (`Host::shutdown` in
//! `rekindled`, the shell's `graceful_shutdown` in the GUI).

use std::path::{Path, PathBuf};

/// What arming the quick wipe reviews (plan E7.1): exactly what a trigger
/// destroys, and that it cannot be undone.
pub const ARM_REVIEW: &str = "the quick wipe. A trigger (the duress passphrase at any \
passphrase prompt, or the panic shortcut) deletes every identity on this device with its \
messages, keys and files, the settings, logs and skins, Rekindle's network storage and the \
window data, and closes Rekindle. It cannot be undone.";

use rekindle_db::paths::DataRoot;

/// A path the wipe could not remove.
#[derive(Debug, thiserror::Error)]
#[error("wipe incomplete: cannot delete {path}: {source}")]
pub struct WipeError {
    pub path: PathBuf,
    pub source: std::io::Error,
}

/// What the node still holds while it runs: Veilid's storage under
/// `state/`, and the node lock beside `state/` in `DataRoot.data`.
const HELD_IN_STATE: &[&str] = &["veilid"];
const HELD_IN_DATA: &[&str] = &["state", rekindle_db::lock::LOCK_FILE];

/// Remove everything in the wipe set the running node does not hold:
/// `config`, `logs`, all of `state/` but Veilid's storage (every identity,
/// file caches, the registry, the layout stamp), everything else in
/// `DataRoot.data` (`skins/`, `file_cache/`), and `bus_dir/bus.pub`.
///
/// # Errors
/// A path that exists could not be removed.
pub fn sweep_root(root: &DataRoot, bus_dir: &Path) -> Result<(), WipeError> {
    destroy_keys(root)?;
    remove(&root.config)?;
    remove(&root.logs)?;
    remove_all_but(&root.state, HELD_IN_STATE)?;
    remove_all_but(&root.data, HELD_IN_DATA)?;
    remove(&bus_dir.join("bus.pub"))
}

/// The keys go first (research cross-check 2026-11-19 §5; NIST SP 800-88
/// Rev. 1 §2.6 "cryptographic erase"): every vault header in `state/`
/// (`{pk}.vault.hdr`, its Argon2id salt and parameters, plan E7.1) is
/// overwritten with zeros, synced and removed before any other file, so a
/// wipe interrupted after this point leaves vaults no passphrase opens,
/// and with them the database keys they hold. The unlocked identity's
/// in-memory keys were released (zeroized) before the sweep
/// (`handle_wipe` → `release_unlock`).
///
/// # Errors
/// A header that exists could not be overwritten or removed.
fn destroy_keys(root: &DataRoot) -> Result<(), WipeError> {
    let entries = match std::fs::read_dir(&root.state) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(WipeError {
                path: root.state.clone(),
                source,
            })
        }
    };
    for entry in entries {
        let path = entry
            .map_err(|source| WipeError {
                path: root.state.clone(),
                source,
            })?
            .path();
        if path.extension().is_some_and(|ext| ext == "hdr") {
            shred(&path)?;
        }
    }
    Ok(())
}

/// Overwrite a header file with zeros, sync it, then remove it.
fn shred(path: &Path) -> Result<(), WipeError> {
    use std::io::Write as _;
    let fail = |source| WipeError {
        path: path.to_path_buf(),
        source,
    };
    let len = std::fs::symlink_metadata(path).map_err(fail)?.len();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(fail)?;
    let len = usize::try_from(len)
        .map_err(|e| fail(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
    let zeros = vec![0u8; len];
    file.write_all(&zeros).map_err(fail)?;
    file.sync_all().map_err(fail)?;
    drop(file);
    remove(path)
}

/// Remove what the node held, once it has stopped: all of `DataRoot.data`
/// (Veilid's storage, the node lock) and the logs again, which `rekindled`
/// wrote until its appender stopped.
///
/// # Errors
/// A path that exists could not be removed.
pub fn finish(root: &DataRoot) -> Result<(), WipeError> {
    remove(&root.data)?;
    remove(&root.logs)
}

/// Remove each of `paths` (the GUI's webview stores,
/// `DataRoot::webview_dirs`), after its webviews are gone.
///
/// # Errors
/// A path that exists could not be removed.
pub fn remove_paths(paths: &[PathBuf]) -> Result<(), WipeError> {
    paths.iter().try_for_each(|path| remove(path))
}

fn remove_all_but(dir: &Path, keep: &[&str]) -> Result<(), WipeError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(WipeError {
                path: dir.to_path_buf(),
                source,
            })
        }
    };
    for entry in entries {
        let entry = entry.map_err(|source| WipeError {
            path: dir.to_path_buf(),
            source,
        })?;
        if keep.iter().any(|k| entry.file_name() == **k) {
            continue;
        }
        remove(&entry.path())?;
    }
    Ok(())
}

/// Remove a file or a directory tree; one that is not there is not an
/// error. A symlink is removed, never followed.
fn remove(path: &Path) -> Result<(), WipeError> {
    let result = match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(e) => Err(e),
    };
    match result {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(WipeError {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(test)]
#[path = "wipe_tests.rs"]
mod tests;

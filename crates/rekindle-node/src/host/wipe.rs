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
    remove(&root.config)?;
    remove(&root.logs)?;
    remove_all_but(&root.state, HELD_IN_STATE)?;
    remove_all_but(&root.data, HELD_IN_DATA)?;
    remove(&bus_dir.join("bus.pub"))
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

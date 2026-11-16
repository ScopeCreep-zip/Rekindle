//! A node's data root, copied and put back (plan E3.2 S7, the equivocation
//! scenario): what a user does who restores a backup of their profile
//! directory. The copy is taken while the node is locked, so no database
//! is open. Veilid keeps its record store under that root too, so the sim's
//! local store for the node is copied and put back with it.

use std::fs;
use std::path::Path;

use crate::harness::SimNode;

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create the copy's directory");
    let mut entries: Vec<_> = fs::read_dir(from)
        .expect("read the data root")
        .map(|entry| entry.expect("a directory entry"))
        .collect();
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let target = to.join(entry.file_name());
        if entry.file_type().expect("an entry type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("copy a data root file");
        }
    }
}

fn clear(dir: &Path) {
    for entry in fs::read_dir(dir).expect("read the data root") {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            fs::remove_dir_all(&path).expect("remove a data root directory");
        } else {
            fs::remove_file(&path).expect("remove a data root file");
        }
    }
}

/// A backup of one node's profile: its data root and the Veilid local
/// record store, which Veilid keeps under that root.
pub struct Backup {
    dir: tempfile::TempDir,
    store: rekindle_netsim::LocalStore,
}

/// A copy of `node`'s data root and local record store (the node must be
/// locked).
pub fn snapshot(sim: &rekindle_netsim::Netsim, node: &SimNode) -> Backup {
    let dir = tempfile::tempdir().expect("temp dir");
    copy_tree(node.root_dir(), dir.path());
    Backup {
        dir,
        store: sim.local_store(node.index),
    }
}

/// Put `backup` back as `node`'s data root and local record store (the
/// node must be locked): everything written since is gone from both.
pub fn restore(sim: &rekindle_netsim::Netsim, node: &SimNode, backup: &Backup) {
    clear(node.root_dir());
    copy_tree(backup.dir.path(), node.root_dir());
    sim.restore_local_store(node.index, backup.store.clone());
}

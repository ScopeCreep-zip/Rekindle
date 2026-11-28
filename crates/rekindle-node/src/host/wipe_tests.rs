//! The quick wipe over a temporary root (never a real data root).

use rekindle_db::paths::DataRoot;

/// A root laid out like a used one, every path the wipe must take.
fn populated(base: &std::path::Path) -> (DataRoot, std::path::PathBuf) {
    let root = DataRoot::under(base);
    root.create_dirs().expect("dirs");
    let write = |p: std::path::PathBuf| {
        std::fs::create_dir_all(p.parent().expect("parent")).expect("parent dir");
        std::fs::write(p, b"x").expect("write");
    };
    write(root.identity_db("pk"));
    write(root.identity_vault("pk"));
    write(root.identity_files("pk").join("blob"));
    write(root.registry());
    write(root.layout_version());
    write(root.veilid().join("table_store"));
    write(root.skins().join("skin.json"));
    write(root.file_cache().join("cached"));
    write(root.data.join(rekindle_db::lock::LOCK_FILE));
    write(root.user_config());
    write(root.config.join("agent-keys/key"));
    write(root.logs.join("rekindled.log"));
    let bus = base.join("run");
    write(bus.join("bus.pub"));
    (root, bus)
}

#[test]
fn the_sweep_leaves_only_what_the_node_holds() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (root, bus) = populated(dir.path());
    super::sweep_root(&root, &bus).expect("sweep");
    assert!(!root.config.exists(), "config");
    assert!(!root.logs.exists(), "logs");
    assert!(!root.skins().exists(), "a swept root has no skins/");
    assert!(!root.file_cache().exists(), "file_cache/");
    assert!(!bus.join("bus.pub").exists(), "bus.pub");
    let mut state: Vec<_> = std::fs::read_dir(&root.state)
        .expect("state")
        .map(|e| e.expect("entry").file_name())
        .collect();
    state.sort();
    assert_eq!(state, ["veilid"], "only Veilid's storage stays in state/");
    assert!(root.data.join(rekindle_db::lock::LOCK_FILE).exists());
}

#[test]
fn finish_leaves_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (root, bus) = populated(dir.path());
    super::sweep_root(&root, &bus).expect("sweep");
    std::fs::create_dir_all(&root.logs).expect("the appender wrote again");
    super::finish(&root).expect("finish");
    assert!(!root.data.exists(), "data");
    assert!(!root.logs.exists(), "logs");
    assert!(!root.config.exists(), "config");
}

#[cfg(unix)]
#[test]
fn a_symlink_is_removed_not_followed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (root, bus) = populated(dir.path());
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&outside).expect("outside");
    std::fs::write(outside.join("keep"), b"x").expect("write");
    std::os::unix::fs::symlink(&outside, root.state.join("link")).expect("symlink");
    super::sweep_root(&root, &bus).expect("sweep");
    assert!(outside.join("keep").exists(), "the link's target survives");
}

/// Keys first (research cross-check 2026-11-19 §5, NIST SP 800-88
/// cryptographic erase): a sweep that stops at the first path it cannot
/// remove has already destroyed every vault header, so no passphrase opens
/// what is left.
#[cfg(unix)]
#[test]
fn an_interrupted_sweep_has_already_destroyed_the_vault_headers() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().expect("tempdir");
    let (root, bus) = populated(dir.path());
    let header = rekindle_vault::header::header_path(&root.identity_vault("pk"));
    std::fs::write(&header, [7u8; 128]).expect("header");
    // `config/` cannot be removed: its subdirectory refuses deletions.
    let stuck = root.config.join("stuck");
    std::fs::create_dir_all(&stuck).expect("stuck");
    std::fs::write(stuck.join("f"), b"x").expect("f");
    std::fs::set_permissions(&stuck, std::fs::Permissions::from_mode(0o500)).expect("chmod");
    let swept = super::sweep_root(&root, &bus);
    std::fs::set_permissions(&stuck, std::fs::Permissions::from_mode(0o700)).expect("chmod");
    assert!(swept.is_err(), "config/ was not removable");
    assert!(
        !header.exists(),
        "the vault header went before anything else"
    );
}

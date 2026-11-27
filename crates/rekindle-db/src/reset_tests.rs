use super::*;

fn root(dir: &Path) -> DataRoot {
    DataRoot::under(dir)
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

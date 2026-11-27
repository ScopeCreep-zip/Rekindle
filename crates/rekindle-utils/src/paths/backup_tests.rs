use super::*;

/// `tmutil isexcluded`'s verdict on `path`.
#[cfg(target_os = "macos")]
fn time_machine_says(path: &Path) -> String {
    let out = std::process::Command::new("tmutil")
        .arg("isexcluded")
        .arg(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[cfg(target_os = "macos")]
#[test]
fn time_machine_reports_state_excluded() {
    // The system temp directory and cargo's `target/` are excluded
    // already, so the root is made in the crate's own directory.
    let base = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let root = DataRoot::under(base.path());
    root.create_dirs().unwrap();
    assert!(time_machine_says(&root.state).starts_with("[Included]"));
    root.exclude_state_from_backup().unwrap();
    let said = time_machine_says(&root.state);
    assert!(said.starts_with("[Excluded]"), "{said}");
    // A file in it is excluded with it; only `state/` is marked.
    let db = root.identity_db("pk");
    std::fs::write(&db, b"x").unwrap();
    assert!(time_machine_says(&db).starts_with("[Excluded]"));
    assert!(time_machine_says(&root.config).starts_with("[Included]"));
}

#[cfg(target_os = "linux")]
#[test]
fn deja_dup_marker_is_in_state() {
    let base = tempfile::tempdir().unwrap();
    let root = DataRoot::under(base.path());
    root.create_dirs().unwrap();
    root.exclude_state_from_backup().unwrap();
    assert!(root.state.join(DEJA_DUP_MARKER).is_file());
    assert!(!root.config.join(DEJA_DUP_MARKER).exists());
}

#[test]
fn marking_twice_is_harmless() {
    let base = tempfile::tempdir().unwrap();
    let root = DataRoot::under(base.path());
    root.create_dirs().unwrap();
    root.exclude_state_from_backup().unwrap();
    root.exclude_state_from_backup().unwrap();
}

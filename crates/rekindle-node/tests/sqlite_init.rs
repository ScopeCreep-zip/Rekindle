//! The Host claims its data root only after SQLite's initialization has run
//! to completion (finding DB1b), so the Veilid node it starts next cannot be
//! the process's first SQLite user and race a keyed open. Its own test
//! binary: a fresh process, where nothing else has initialized SQLite.

#[test]
fn claiming_a_root_initializes_sqlite_first() {
    assert!(!rekindle_db::sqlite_initialized());
    let dir = tempfile::tempdir().unwrap();
    let _claimed =
        rekindle_node::host::claim(rekindle_db::paths::DataRoot::under(dir.path())).unwrap();
    assert!(rekindle_db::sqlite_initialized());
}

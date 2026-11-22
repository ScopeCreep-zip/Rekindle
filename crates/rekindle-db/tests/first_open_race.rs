//! The process's first SQLCipher opens, racing (finding DB1b).
//!
//! SQLite marks itself initialized (`isInit = 1`) inside its main mutex
//! and runs `SQLITE_EXTRA_INIT` (SQLCipher's `sqlcipher_extra_init`) only
//! after leaving it; a second thread's `sqlite3_initialize` returns at the
//! `isInit` check in between, and its `PRAGMA key` reaches
//! `sqlcipherCodecAttach` while `sqlcipher_init` is still 0, which answers
//! `sqlcipher_init_error`'s initial `SQLITE_ERROR` (libsqlite3-sys 0.37
//! `sqlcipher/sqlite3.c`: `sqlite3_initialize`, `sqlcipherCodecAttach`).
//! `rusqlite` 0.39 does not serialize the first initialization.
//!
//! This file holds one test so that its process has opened no SQLite
//! connection before the race: the threads' opens are the first ones.

use std::sync::{Arc, Barrier};

const THREADS: usize = 8;

#[test]
fn the_first_keyed_opens_of_a_process_all_succeed() {
    let dir = tempfile::tempdir().unwrap();
    let start = Arc::new(Barrier::new(THREADS));
    let opens: Vec<_> = (0..THREADS)
        .map(|i| {
            let path = dir.path().join(format!("pk{i}.db"));
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                rekindle_db::open(&path, &[7; 32]).map(drop)
            })
        })
        .collect();
    for (i, open) in opens.into_iter().enumerate() {
        let result = open.join().unwrap();
        assert!(result.is_ok(), "open {i} failed: {result:?}");
    }
}

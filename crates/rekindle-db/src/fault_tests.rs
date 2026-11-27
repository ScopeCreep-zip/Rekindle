use super::*;

fn failure(code: i32) -> rekindle_asql::Error {
    rekindle_asql::Error::Error(rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(code),
        None,
    ))
}

#[test]
fn a_constraint_or_size_failure_is_the_inputs_and_the_rest_ours() {
    assert!(input_fault(&failure(rusqlite::ffi::SQLITE_CONSTRAINT)));
    assert!(input_fault(&failure(
        rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
    )));
    assert!(input_fault(&failure(rusqlite::ffi::SQLITE_TOOBIG)));
    for ours in [
        rusqlite::ffi::SQLITE_BUSY,
        rusqlite::ffi::SQLITE_LOCKED,
        rusqlite::ffi::SQLITE_IOERR,
        rusqlite::ffi::SQLITE_FULL,
        rusqlite::ffi::SQLITE_CORRUPT,
    ] {
        assert!(!input_fault(&failure(ours)), "{ours} is ours");
    }
    assert!(!input_fault(&rekindle_asql::Error::ConnectionClosed));
}

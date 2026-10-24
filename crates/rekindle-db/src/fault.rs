//! Whose fault a failed store is (review RV5 M2). A store that fails on
//! what the caller wrote (a constraint, a value too big, a value that does
//! not convert) is the input's: when the input is a peer's frame it is a
//! judged refusal. Anything else (busy, locked, I/O, a full disk, a
//! corrupt file, a closed connection) is ours, and a peer is told to try
//! again later.

/// Whether `e` is a fault of the values the store was given, not of the
/// database.
#[must_use]
pub fn input_fault(e: &crate::CallError) -> bool {
    match e {
        rekindle_asql::Error::Error(e) => rusqlite_input_fault(e),
        _ => false,
    }
}

fn rusqlite_input_fault(e: &rusqlite::Error) -> bool {
    match e {
        rusqlite::Error::SqliteFailure(failure, _) => matches!(
            failure.code,
            rusqlite::ErrorCode::ConstraintViolation | rusqlite::ErrorCode::TooBig
        ),
        rusqlite::Error::ToSqlConversionFailure(_) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
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
}

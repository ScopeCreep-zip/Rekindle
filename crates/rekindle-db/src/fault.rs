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
#[path = "fault_tests.rs"]
mod tests;

//! The desktop's lenient row helpers. Opening, the schema and the handle
//! live in `rekindle-db`; the open database is `AppState.db` (plan C6).

/// Extract a `String` column by name, returning `""` on any failure.
pub fn get_str(row: &rusqlite::Row<'_>, col: &str) -> String {
    row.get::<_, String>(col).unwrap_or_default()
}

/// Extract an optional `String` column by name.
pub fn get_str_opt(row: &rusqlite::Row<'_>, col: &str) -> Option<String> {
    row.get::<_, Option<String>>(col).ok().flatten()
}

/// Extract an `i64` column by name, returning `0` on any failure.
pub fn get_i64(row: &rusqlite::Row<'_>, col: &str) -> i64 {
    row.get::<_, i64>(col).unwrap_or_default()
}

/// Current UNIX timestamp in milliseconds.
pub fn timestamp_now() -> i64 {
    rekindle_utils::timestamp_ms_i64()
}

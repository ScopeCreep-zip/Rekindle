//! The desktop's time helper for rows it writes. Opening, the schema, the
//! handle and the strict row readers (`rekindle_db::row`, plan D2) live in
//! `rekindle-db`; the open database is `AppState.db` (plan C6).

/// Current UNIX timestamp in milliseconds.
pub fn timestamp_now() -> i64 {
    rekindle_utils::timestamp_ms_i64()
}

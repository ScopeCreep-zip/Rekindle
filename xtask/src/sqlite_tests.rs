use super::is_sql_shaped;

#[test]
fn sql_shapes() {
    for s in [
        "SELECT display_name FROM community_members WHERE owner_key = ?1",
        "INSERT OR IGNORE INTO friends (owner_key) VALUES (?1)",
        "UPDATE communities SET name = ?1",
        "DELETE FROM identity WHERE public_key = ?1",
        "CREATE TABLE IF NOT EXISTS t (a TEXT)",
        "CREATE VIRTUAL TABLE messages_fts USING fts5(body)",
        "PRAGMA journal_mode=WAL;",
    ] {
        assert!(is_sql_shaped(s), "{s}");
    }
    for s in [
        "select a friend from the list",
        "failed to update the friend list",
        "Delete from your device?",
        "SELECT",
        "the INSERT key",
    ] {
        assert!(!is_sql_shaped(s), "{s}");
    }
}

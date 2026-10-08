use rusqlite::Connection;

use crate::error::VaultError;

/// Create the `entries` table on first open. Idempotent — safe to call
/// every open; CREATE TABLE IF NOT EXISTS is a no-op once the schema
/// exists.
pub fn ensure(conn: &Connection) -> Result<(), VaultError> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS entries (
            namespace TEXT NOT NULL,
            key TEXT NOT NULL,
            nonce BLOB NOT NULL,
            ciphertext BLOB NOT NULL,
            PRIMARY KEY (namespace, key)
        );
        ",
    )?;
    Ok(())
}

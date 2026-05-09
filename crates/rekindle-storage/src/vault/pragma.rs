//! SQLCipher PRAGMA configuration.
//!
//! Applied on every vault open. Matches open-sesame's SqlCipherStore pattern.

use rusqlite::Connection;
use zeroize::Zeroizing;

use crate::error::{StorageError, StorageResult};

pub fn configure(conn: &Connection, vault_key: &Zeroizing<[u8; 32]>) -> StorageResult<()> {
    let key_hex = hex::encode(vault_key.as_ref());

    // SQLCipher requires PRAGMA key as the very first statement after open.
    conn.execute_batch(&format!("PRAGMA key = \"x'{key_hex}'\";"))
        .map_err(|e| StorageError::VaultOpenFailed {
            reason: format!("PRAGMA key: {e}"),
        })?;

    conn.execute_batch(
        "PRAGMA cipher_page_size = 4096;
         PRAGMA kdf_iter = 256000;
         PRAGMA cipher_memory_security = ON;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = FULL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;",
    )
    .map_err(|e| StorageError::VaultOpenFailed {
        reason: format!("PRAGMA config: {e}"),
    })?;

    Ok(())
}

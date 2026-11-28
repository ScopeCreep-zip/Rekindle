//! Identity export (plan D1): the identity vault's bytes and its header's,
//! without the entries that belong to this device only (ADR 0022 rule 4,
//! [`VaultKey::is_device_scoped`]). The live vault is never changed: the
//! entries are deleted from a scratch copy beside it, which is removed again.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};

use super::{write_private_new, VaultStore};
use crate::{VaultError, VaultKey};

/// A scratch copy of the vault, removed with its rollback journal when
/// dropped, whatever the export's outcome.
struct ScratchCopy(PathBuf);

impl Drop for ScratchCopy {
    fn drop(&mut self) {
        let mut journal = self.0.clone().into_os_string();
        journal.push("-journal");
        for path in [self.0.as_path(), Path::new(&journal)] {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl VaultStore {
    /// The vault file's bytes and its header's, for an identity archive,
    /// with every device-scoped entry removed (ADR 0022 rule 4: a device key
    /// never travels with an export; the importing device mints its own,
    /// [`crate::device::open`]).
    ///
    /// Read under the connection lock: every write goes through it, and the
    /// vault journals by rollback, so the file copied is a committed state.
    /// The copy is attached to this connection, which keys it (SQLCipher: an
    /// `ATTACH` without `KEY` uses the main database's key); the entries are
    /// deleted under `secure_delete` and the copy vacuumed, so no freed page
    /// keeps them.
    ///
    /// # Errors
    /// I/O or SQLite.
    pub fn export_files(&self) -> Result<(Vec<u8>, Vec<u8>), VaultError> {
        let conn = self.conn.lock();
        let header = std::fs::read(crate::header::header_path(&self.path))?;
        let scratch = ScratchCopy(scratch_path(&self.path));
        write_private_new(&scratch.0, &std::fs::read(&self.path)?)?;
        let target = scratch
            .0
            .to_str()
            .ok_or_else(|| VaultError::Schema("vault path is not UTF-8".into()))?;
        conn.execute("ATTACH DATABASE ?1 AS export", params![target])?;
        let stripped = strip_device_scoped(&conn);
        let detached = conn.execute_batch("DETACH DATABASE export");
        stripped?;
        detached?;
        let vault = std::fs::read(&scratch.0)?;
        Ok((vault, header))
    }
}

/// `{vault}.export-<random>`: beside the vault, so in its owner-only
/// directory and on its filesystem.
fn scratch_path(vault: &Path) -> PathBuf {
    let mut nonce = [0u8; 8];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut nonce);
    let mut name = vault.as_os_str().to_owned();
    name.push(format!(".export-{}", hex::encode(nonce)));
    PathBuf::from(name)
}

/// Delete the attached copy's device-scoped entries, leaving no freed page
/// that holds them.
fn strip_device_scoped(conn: &Connection) -> Result<(), VaultError> {
    conn.execute_batch("PRAGMA export.secure_delete = ON")?;
    let rows: Vec<(String, String)> = {
        let mut stmt = conn.prepare("SELECT namespace, key FROM export.entries")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    for (namespace, key) in rows.iter().filter(|(namespace, key)| {
        VaultKey::parse(namespace, key).is_some_and(|k| k.is_device_scoped())
    }) {
        conn.execute(
            "DELETE FROM export.entries WHERE namespace = ?1 AND key = ?2",
            params![namespace, key],
        )?;
    }
    conn.execute_batch("VACUUM export")?;
    Ok(())
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;

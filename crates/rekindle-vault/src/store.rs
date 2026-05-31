use std::path::{Path, PathBuf};

use aes_gcm::{aead::Aead, AeadCore, Aes256Gcm, KeyInit};
use parking_lot::Mutex;
use rand::RngCore;
use rusqlite::{params, Connection, OptionalExtension};
use zeroize::Zeroizing;

use crate::error::VaultError;
use crate::schema;

/// SQLCipher + per-entry AES-256-GCM keystore.
///
/// One `VaultStore` corresponds to one `.vault` file on disk plus its
/// sidecar `.vault.salt` file. Open with [`VaultStore::open`]; the same
/// passphrase is used to re-derive both layer keys.
pub struct VaultStore {
    conn: Mutex<Connection>,
    entry_key: Zeroizing<[u8; 32]>,
    path: PathBuf,
}

impl VaultStore {
    /// Open the vault at `path`, creating it if absent. The 32-byte salt
    /// lives in a sidecar file `{path}.salt`; on first open a random salt
    /// is written. Subsequent opens reuse it.
    pub fn open(path: &Path, passphrase: &str) -> Result<Self, VaultError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let salt_path = salt_sidecar(path);
        let salt = load_or_generate_salt(&salt_path)?;
        let (sqlcipher_key, entry_key) = derive_two_keys(passphrase, &salt)?;

        let conn = Connection::open(path)?;
        let key_pragma = format!("x'{}'", hex::encode(*sqlcipher_key));
        conn.pragma_update(None, "key", key_pragma)?;
        conn.pragma_update(None, "cipher_page_size", 4096_i64)?;

        // First query forces SQLCipher to validate the key by reading the
        // header — wrong passphrase fails here with a SqliteFailure error.
        conn.query_row("PRAGMA cipher_version;", [], |_| Ok(()))
            .map_err(|e| {
                VaultError::Schema(format!(
                    "SQLCipher key validation failed (wrong passphrase or corrupt vault): {e}"
                ))
            })?;

        schema::ensure(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            entry_key,
            path: path.to_path_buf(),
        })
    }

    /// On-disk path of this vault file (the SQLCipher database — the salt
    /// sidecar lives at `{path}.salt`).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Insert-or-replace an entry under (`namespace`, `key`). The value is
    /// sealed with AES-256-GCM under the per-entry key before being stored.
    pub fn put(&self, namespace: &str, key: &str, value: &[u8]) -> Result<(), VaultError> {
        let (nonce, ct) = seal_aes_gcm(&self.entry_key, value)?;
        self.conn.lock().execute(
            "INSERT OR REPLACE INTO entries (namespace, key, nonce, ciphertext) VALUES (?1, ?2, ?3, ?4)",
            params![namespace, key, nonce, ct],
        )?;
        Ok(())
    }

    /// Look up and decrypt the entry under (`namespace`, `key`). Returns
    /// `None` if the row doesn't exist.
    pub fn get(
        &self,
        namespace: &str,
        key: &str,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, VaultError> {
        let conn = self.conn.lock();
        let row: Option<(Vec<u8>, Vec<u8>)> = conn
            .query_row(
                "SELECT nonce, ciphertext FROM entries WHERE namespace = ?1 AND key = ?2",
                params![namespace, key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match row {
            Some((nonce, ct)) => Ok(Some(open_aes_gcm(&self.entry_key, &nonce, &ct)?)),
            None => Ok(None),
        }
    }

    /// Remove the entry under (`namespace`, `key`). Idempotent — no error
    /// if the row didn't exist.
    pub fn delete(&self, namespace: &str, key: &str) -> Result<(), VaultError> {
        self.conn.lock().execute(
            "DELETE FROM entries WHERE namespace = ?1 AND key = ?2",
            params![namespace, key],
        )?;
        Ok(())
    }

    /// Whether (`namespace`, `key`) has a stored entry.
    pub fn key_exists(&self, namespace: &str, key: &str) -> Result<bool, VaultError> {
        let conn = self.conn.lock();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM entries WHERE namespace = ?1 AND key = ?2",
            params![namespace, key],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    /// Diagnostic: number of rows in the entries table.
    pub fn entry_count(&self) -> Result<usize, VaultError> {
        let conn = self.conn.lock();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))?;
        Ok(usize::try_from(count).unwrap_or(0))
    }
}

fn salt_sidecar(vault_path: &Path) -> PathBuf {
    let mut sidecar = vault_path.as_os_str().to_owned();
    sidecar.push(".salt");
    PathBuf::from(sidecar)
}

fn load_or_generate_salt(salt_path: &Path) -> Result<[u8; 32], VaultError> {
    if salt_path.exists() {
        let bytes = std::fs::read(salt_path)?;
        if bytes.len() != 32 {
            return Err(VaultError::Schema(format!(
                "salt sidecar {} has length {} (expected 32)",
                salt_path.display(),
                bytes.len()
            )));
        }
        let mut salt = [0u8; 32];
        salt.copy_from_slice(&bytes);
        Ok(salt)
    } else {
        let mut salt = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut salt);
        std::fs::write(salt_path, salt)?;
        Ok(salt)
    }
}

fn derive_two_keys(
    passphrase: &str,
    salt: &[u8; 32],
) -> Result<(Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>), VaultError> {
    // Argon2id default params: m_cost=19456 KiB, t_cost=2, p_cost=1 — OWASP
    // 2023 minimum for interactive logins. 64-byte master output.
    let mut master = Zeroizing::new([0u8; 64]);
    argon2::Argon2::default()
        .hash_password_into(passphrase.as_bytes(), salt, &mut *master)
        .map_err(|e| VaultError::Kdf(e.to_string()))?;
    let sqlcipher = Zeroizing::new(blake3::derive_key("rekindle v1 vault-sqlcipher", &*master));
    let entry = Zeroizing::new(blake3::derive_key("rekindle v1 vault-entry-gcm", &*master));
    Ok((sqlcipher, entry))
}

fn seal_aes_gcm(key: &[u8; 32], plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>), VaultError> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| VaultError::Aead(e.to_string()))?;
    let nonce = Aes256Gcm::generate_nonce(&mut rand::rngs::OsRng);
    let ct = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| VaultError::Aead(e.to_string()))?;
    Ok((nonce.to_vec(), ct))
}

fn open_aes_gcm(
    key: &[u8; 32],
    nonce: &[u8],
    ct: &[u8],
) -> Result<Zeroizing<Vec<u8>>, VaultError> {
    if nonce.len() != 12 {
        return Err(VaultError::Aead(format!(
            "nonce length {} (expected 12)",
            nonce.len()
        )));
    }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| VaultError::Aead(e.to_string()))?;
    let pt = cipher
        .decrypt(nonce.into(), ct)
        .map_err(|e| VaultError::Aead(e.to_string()))?;
    Ok(Zeroizing::new(pt))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fresh_path() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("alice.vault");
        (dir, path)
    }

    #[test]
    fn round_trip_put_get_delete() {
        let (_dir, path) = fresh_path();
        let vault = VaultStore::open(&path, "passphrase-test-1").unwrap();
        vault.put("ns", "k", b"hello world").unwrap();
        let got = vault.get("ns", "k").unwrap().unwrap();
        assert_eq!(&*got, b"hello world");
        vault.delete("ns", "k").unwrap();
        assert!(vault.get("ns", "k").unwrap().is_none());
    }

    #[test]
    fn reopen_same_passphrase_decrypts() {
        let (_dir, path) = fresh_path();
        {
            let vault = VaultStore::open(&path, "passphrase-test-2").unwrap();
            vault.put("ns", "k", b"persisted").unwrap();
        }
        let vault = VaultStore::open(&path, "passphrase-test-2").unwrap();
        let got = vault.get("ns", "k").unwrap().unwrap();
        assert_eq!(&*got, b"persisted");
    }

    #[test]
    fn wrong_passphrase_rejected() {
        let (_dir, path) = fresh_path();
        {
            let vault = VaultStore::open(&path, "right-passphrase").unwrap();
            vault.put("ns", "k", b"secret").unwrap();
        }
        let result = VaultStore::open(&path, "wrong-passphrase");
        assert!(result.is_err(), "wrong passphrase should be rejected");
    }

    #[test]
    fn ciphertext_differs_for_same_plaintext() {
        // Each put() uses a fresh nonce → ciphertexts must differ.
        let (_dir, path) = fresh_path();
        let vault = VaultStore::open(&path, "pp").unwrap();
        vault.put("ns", "a", b"same").unwrap();
        vault.put("ns", "b", b"same").unwrap();
        let conn = vault.conn.lock();
        let mut stmt = conn
            .prepare("SELECT ciphertext FROM entries ORDER BY key")
            .unwrap();
        let mut rows = stmt.query([]).unwrap();
        let ct_a: Vec<u8> = rows.next().unwrap().unwrap().get(0).unwrap();
        let ct_b: Vec<u8> = rows.next().unwrap().unwrap().get(0).unwrap();
        assert_ne!(ct_a, ct_b, "AES-GCM nonces must differ per put");
    }

    #[test]
    fn key_exists_works() {
        let (_dir, path) = fresh_path();
        let vault = VaultStore::open(&path, "pp").unwrap();
        assert!(!vault.key_exists("ns", "k").unwrap());
        vault.put("ns", "k", b"v").unwrap();
        assert!(vault.key_exists("ns", "k").unwrap());
    }
}

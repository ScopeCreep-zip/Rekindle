use std::path::{Path, PathBuf};

use aes_gcm::{aead::Aead, AeadCore, Aes256Gcm, KeyInit};
use parking_lot::Mutex;
use rekindle_types::domains;
use rusqlite::{params, Connection, OptionalExtension};
use zeroize::Zeroizing;

use crate::error::VaultError;
use crate::key::VaultKey;
use crate::schema;

/// The SQLCipher key of the identity's database (plan D1), derived from the
/// vault passphrase, so unlocking the vault opens the database too.
pub struct AppDbKey(Zeroizing<[u8; 32]>);

impl AppDbKey {
    /// The raw key, for `PRAGMA key`.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for AppDbKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AppDbKey(<redacted>)")
    }
}

/// SQLCipher + per-entry AES-256-GCM keystore.
///
/// One `VaultStore` is one `.vault` file plus its `.hdr` (Argon2id
/// parameters and salt, [`crate::header`]). [`VaultStore::create`] makes a
/// new one; [`VaultStore::open`] never creates anything.
pub struct VaultStore {
    conn: Mutex<Connection>,
    entry_key: Zeroizing<[u8; 32]>,
    path: PathBuf,
}

/// The three keys a passphrase unlocks.
struct Keys {
    sqlcipher: Zeroizing<[u8; 32]>,
    entry: Zeroizing<[u8; 32]>,
    app_db: AppDbKey,
}

impl VaultStore {
    /// Create a new vault at `path` (and its `.hdr`), owner-only.
    ///
    /// # Errors
    /// [`VaultError::AlreadyExists`] when a vault or header is there; I/O,
    /// KDF or SQLite failures (the header is removed again on failure).
    pub fn create(path: &Path, passphrase: &str) -> Result<(Self, AppDbKey), VaultError> {
        if path.exists() {
            return Err(VaultError::AlreadyExists);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let header = crate::header::VaultHeader::fresh();
        crate::header::write_new(path, &header)?;
        let created = Self::open_with(path, passphrase, &header).and_then(|opened| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            }
            Ok(opened)
        });
        if created.is_err() {
            let _ = std::fs::remove_file(crate::header::header_path(path));
            let _ = std::fs::remove_file(path);
        }
        created
    }

    /// Open the existing vault at `path`. Nothing is created.
    ///
    /// # Errors
    /// [`VaultError::NotFound`], [`VaultError::HeaderMissing`],
    /// [`VaultError::WrongPassphrase`], or I/O, KDF and SQLite failures.
    pub fn open(path: &Path, passphrase: &str) -> Result<(Self, AppDbKey), VaultError> {
        if !path.exists() {
            return Err(VaultError::NotFound);
        }
        let header = crate::header::read(path)?;
        Self::open_with(path, passphrase, &header)
    }

    fn open_with(
        path: &Path,
        passphrase: &str,
        header: &crate::header::VaultHeader,
    ) -> Result<(Self, AppDbKey), VaultError> {
        let keys = derive_keys(passphrase, header)?;
        let conn = Connection::open(path)?;
        let key_pragma = format!("x'{}'", hex::encode(*keys.sqlcipher));
        // Keying, then the first read, which makes SQLCipher decrypt the
        // header. A key that does not decrypt it surfaces as NOMEM or
        // NOTADB (`VaultError::WrongPassphrase` says why).
        conn.pragma_update(None, "key", key_pragma)
            .and_then(|()| conn.pragma_update(None, "cipher_page_size", 4096_i64))
            .and_then(|()| {
                conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
                    r.get::<_, i64>(0)
                })
            })
            .map_err(VaultError::Sqlite)
            .and_then(|_| schema::ensure(&conn))
            .map_err(wrong_key_or)?;
        Ok((
            Self {
                conn: Mutex::new(conn),
                entry_key: keys.entry,
                path: path.to_path_buf(),
            },
            keys.app_db,
        ))
    }

    /// Remove the vault at `path` and its header. Missing files are not an
    /// error.
    ///
    /// # Errors
    /// I/O.
    pub fn delete_files(path: &Path) -> Result<(), VaultError> {
        for p in [path.to_path_buf(), crate::header::header_path(path)] {
            match std::fs::remove_file(&p) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    /// The vault file's bytes and its header's, for an identity archive.
    /// Read under the connection lock: every write goes through it, and the
    /// vault journals by rollback, so the file is a committed state.
    ///
    /// # Errors
    /// I/O.
    pub fn export_files(&self) -> Result<(Vec<u8>, Vec<u8>), VaultError> {
        let _conn = self.conn.lock();
        let vault = std::fs::read(&self.path)?;
        let header = std::fs::read(crate::header::header_path(&self.path))?;
        Ok((vault, header))
    }

    /// Install a vault and its header from an archive at `path`. Refuses to
    /// replace either; nothing is opened (the caller opens it to verify).
    ///
    /// # Errors
    /// [`VaultError::AlreadyExists`], a header that does not parse, or I/O
    /// (the files written are removed again).
    pub fn import_files(path: &Path, vault: &[u8], header: &[u8]) -> Result<(), VaultError> {
        let parsed = crate::header::VaultHeader::decode(header)?;
        if path.exists() {
            return Err(VaultError::AlreadyExists);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        crate::header::write_new(path, &parsed)?;
        let written = write_private_new(path, vault);
        if written.is_err() {
            let _ = std::fs::remove_file(crate::header::header_path(path));
        }
        written
    }

    /// On-disk path of this vault file (the SQLCipher database — the salt
    /// sidecar lives at `{path}.salt`).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Insert-or-replace the entry addressed by `vault_key`. The value is
    /// sealed with AES-256-GCM under the per-entry key before being stored.
    pub fn put(&self, vault_key: &VaultKey, value: &[u8]) -> Result<(), VaultError> {
        let namespace = vault_key.namespace();
        let key = vault_key.key();
        let (nonce, ct) = seal_aes_gcm(&self.entry_key, &aad(namespace, &key), value)?;
        self.conn.lock().execute(
            "INSERT OR REPLACE INTO entries (namespace, key, nonce, ciphertext) VALUES (?1, ?2, ?3, ?4)",
            params![namespace, key.as_ref(), nonce, ct],
        )?;
        Ok(())
    }

    /// Look up and decrypt the entry addressed by `vault_key`. Returns
    /// `None` if the row doesn't exist.
    pub fn get(&self, vault_key: &VaultKey) -> Result<Option<Zeroizing<Vec<u8>>>, VaultError> {
        let namespace = vault_key.namespace();
        let key = vault_key.key();
        let conn = self.conn.lock();
        let row: Option<(Vec<u8>, Vec<u8>)> = conn
            .query_row(
                "SELECT nonce, ciphertext FROM entries WHERE namespace = ?1 AND key = ?2",
                params![namespace, key.as_ref()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match row {
            Some((nonce, ct)) => Ok(Some(open_aes_gcm(
                &self.entry_key,
                &aad(namespace, &key),
                &nonce,
                &ct,
            )?)),
            None => Ok(None),
        }
    }

    /// Remove the entry addressed by `vault_key`. Idempotent — no error
    /// if the row didn't exist.
    pub fn delete(&self, vault_key: &VaultKey) -> Result<(), VaultError> {
        let namespace = vault_key.namespace();
        let key = vault_key.key();
        self.conn.lock().execute(
            "DELETE FROM entries WHERE namespace = ?1 AND key = ?2",
            params![namespace, key.as_ref()],
        )?;
        Ok(())
    }

    /// Whether `vault_key` has a stored entry.
    pub fn key_exists(&self, vault_key: &VaultKey) -> Result<bool, VaultError> {
        let namespace = vault_key.namespace();
        let key = vault_key.key();
        let conn = self.conn.lock();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM entries WHERE namespace = ?1 AND key = ?2",
            params![namespace, key.as_ref()],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    /// Every stored key in `namespace` that parses as a [`VaultKey`].
    ///
    /// # Errors
    /// SQLite.
    pub fn list(&self, namespace: &str) -> Result<Vec<VaultKey>, VaultError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT key FROM entries WHERE namespace = ?1 ORDER BY key")?;
        let keys = stmt
            .query_map(params![namespace], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(keys
            .iter()
            .filter_map(|k| VaultKey::parse(namespace, k))
            .collect())
    }

    /// Diagnostic: number of rows in the entries table.
    pub fn entry_count(&self) -> Result<usize, VaultError> {
        let conn = self.conn.lock();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))?;
        Ok(usize::try_from(count).unwrap_or(0))
    }
}

/// Write `bytes` to a new owner-only file at `path`, synced.
fn write_private_new(path: &Path, bytes: &[u8]) -> Result<(), VaultError> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => VaultError::AlreadyExists,
        _ => e.into(),
    })?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    if let Err(e) = written {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(e.into());
    }
    Ok(())
}

/// The AAD that binds an entry to its address: `namespace ‖ 0x00 ‖ key`,
/// so a ciphertext moved to another key does not open.
fn aad(namespace: &str, key: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(namespace.len() + 1 + key.len());
    out.extend_from_slice(namespace.as_bytes());
    out.push(0);
    out.extend_from_slice(key.as_bytes());
    out
}

fn derive_keys(passphrase: &str, header: &crate::header::VaultHeader) -> Result<Keys, VaultError> {
    let argon = argon2::Argon2::new(
        argon2::Algorithm::Argon2id,
        argon2::Version::V0x13,
        header.params()?,
    );
    let mut master = Zeroizing::new(vec![0u8; header.output_len as usize]);
    argon
        .hash_password_into(passphrase.as_bytes(), &header.salt, &mut master)
        .map_err(|e| VaultError::Kdf(e.to_string()))?;
    let derive = |label: &str| Zeroizing::new(blake3::derive_key(label, &master));
    Ok(Keys {
        sqlcipher: derive(domains::VAULT_SQLCIPHER_KEY),
        entry: derive(domains::VAULT_ENTRY_KEY),
        app_db: AppDbKey(derive(domains::VAULT_APP_DB_KEY)),
    })
}

fn seal_aes_gcm(
    key: &[u8; 32],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<(Vec<u8>, Vec<u8>), VaultError> {
    use aes_gcm::aead::Payload;
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| VaultError::Aead(e.to_string()))?;
    let nonce = Aes256Gcm::generate_nonce(&mut rand::rngs::OsRng);
    let ct = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|e| VaultError::Aead(e.to_string()))?;
    Ok((nonce.to_vec(), ct))
}

fn open_aes_gcm(
    key: &[u8; 32],
    aad: &[u8],
    nonce: &[u8],
    ct: &[u8],
) -> Result<Zeroizing<Vec<u8>>, VaultError> {
    use aes_gcm::aead::Payload;
    if nonce.len() != 12 {
        return Err(VaultError::Aead(format!(
            "nonce length {} (expected 12)",
            nonce.len()
        )));
    }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| VaultError::Aead(e.to_string()))?;
    let pt = cipher
        .decrypt(nonce.into(), Payload { msg: ct, aad })
        .map_err(|e| VaultError::Aead(e.to_string()))?;
    Ok(Zeroizing::new(pt))
}

/// A keying-phase error: a page that will not decrypt under this key is
/// [`VaultError::WrongPassphrase`]; anything else stands.
fn wrong_key_or(error: VaultError) -> VaultError {
    match &error {
        VaultError::Sqlite(e)
            if matches!(
                e.sqlite_error_code(),
                Some(rusqlite::ErrorCode::OutOfMemory | rusqlite::ErrorCode::NotADatabase)
            ) =>
        {
            VaultError::WrongPassphrase
        }
        _ => error,
    }
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
    fn exported_files_import_and_open_with_the_same_passphrase() {
        let (dir, path) = fresh_path();
        let (vault, key) = VaultStore::create(&path, "pp").unwrap();
        vault.put(&VaultKey::IdentityEd25519, &[3u8; 32]).unwrap();
        let (bytes, header) = vault.export_files().unwrap();
        let copy = dir.path().join("copy.vault");
        VaultStore::import_files(&copy, &bytes, &header).unwrap();
        assert!(matches!(
            VaultStore::import_files(&copy, &bytes, &header),
            Err(VaultError::AlreadyExists)
        ));
        let (opened, copy_key) = VaultStore::open(&copy, "pp").unwrap();
        assert_eq!(copy_key.as_bytes(), key.as_bytes());
        assert_eq!(
            &*opened.get(&VaultKey::IdentityEd25519).unwrap().unwrap(),
            &[3u8; 32]
        );
        assert!(matches!(
            VaultStore::import_files(&dir.path().join("bad.vault"), &bytes, b"junk"),
            Err(VaultError::Schema(_))
        ));
    }

    #[test]
    fn create_then_open_and_open_never_creates() {
        let (_dir, path) = fresh_path();
        assert!(matches!(
            VaultStore::open(&path, "pp"),
            Err(VaultError::NotFound)
        ));
        assert!(!path.exists(), "open created nothing");
        let (vault, key) = VaultStore::create(&path, "pp").unwrap();
        vault.put(&VaultKey::AuditTail, b"persisted").unwrap();
        drop(vault);
        assert!(matches!(
            VaultStore::create(&path, "pp"),
            Err(VaultError::AlreadyExists)
        ));
        let (vault, again) = VaultStore::open(&path, "pp").unwrap();
        assert_eq!(
            &*vault.get(&VaultKey::AuditTail).unwrap().unwrap(),
            b"persisted"
        );
        assert_eq!(key.as_bytes(), again.as_bytes(), "the app-db key is stable");
    }

    #[test]
    fn a_wrong_passphrase_is_reported_as_one() {
        let (_dir, path) = fresh_path();
        drop(VaultStore::create(&path, "right").unwrap());
        assert!(matches!(
            VaultStore::open(&path, "wrong"),
            Err(VaultError::WrongPassphrase)
        ));
        assert!(VaultStore::open(&path, "right").is_ok());
    }

    #[test]
    fn a_missing_header_is_reported() {
        let (_dir, path) = fresh_path();
        drop(VaultStore::create(&path, "pp").unwrap());
        std::fs::remove_file(crate::header::header_path(&path)).unwrap();
        assert!(matches!(
            VaultStore::open(&path, "pp"),
            Err(VaultError::HeaderMissing)
        ));
    }

    #[test]
    fn round_trip_put_get_delete_and_list() {
        let (_dir, path) = fresh_path();
        let (vault, _) = VaultStore::create(&path, "pp").unwrap();
        let k = VaultKey::AuditMacKey;
        vault.put(&k, b"hello world").unwrap();
        assert_eq!(&*vault.get(&k).unwrap().unwrap(), b"hello world");
        let mek = |community: &str| VaultKey::CommunityMek {
            community: community.into(),
        };
        vault.put(&mek("c7"), b"x").unwrap();
        vault.put(&mek("c9"), b"y").unwrap();
        let listed = vault.list("communities").unwrap();
        assert_eq!(listed, vec![mek("c7"), mek("c9")]);
        vault.delete(&k).unwrap();
        assert!(vault.get(&k).unwrap().is_none());
    }

    /// An entry's ciphertext is bound to its address: moved under another
    /// key it does not open.
    #[test]
    fn an_entry_moved_to_another_key_does_not_open() {
        let (_dir, path) = fresh_path();
        let (vault, _) = VaultStore::create(&path, "pp").unwrap();
        let mek = |community: &str| VaultKey::CommunityMek {
            community: community.into(),
        };
        vault.put(&mek("c1"), b"one").unwrap();
        {
            let conn = vault.conn.lock();
            conn.execute(
                "UPDATE entries SET key = ?1 WHERE key = ?2",
                params![mek("c2").key().as_ref(), mek("c1").key().as_ref()],
            )
            .unwrap();
        }
        assert!(vault.get(&mek("c2")).is_err());
    }

    #[test]
    fn delete_files_removes_the_vault_and_its_header() {
        let (_dir, path) = fresh_path();
        drop(VaultStore::create(&path, "pp").unwrap());
        VaultStore::delete_files(&path).unwrap();
        assert!(!path.exists());
        assert!(!crate::header::header_path(&path).exists());
        VaultStore::delete_files(&path).unwrap();
    }
}

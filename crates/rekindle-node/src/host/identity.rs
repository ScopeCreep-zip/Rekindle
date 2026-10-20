//! The identity lifecycle, one implementation for every host (plan D1).
//!
//! An identity is three files and one directory under `DataRoot.state`
//! and one registry entry: the vault `{pk}.vault` (+ `.hdr`), the database
//! `{pk}.db` (SQLCipher, keyed by the vault-derived `AppDbKey`), its file
//! cache `{pk}.files/` (keyed-hash names, plan E3.0a), and its row in
//! `identities.json`. [`create`] and [`unlock`] install the vault and the
//! database into the host's [`Stores`]; [`lock`] takes them out again. The
//! 1:1 ratchet's state is in the database (`rekindle_db::ratchet`, plan
//! E2.3).
//!
//! Every wrong passphrase — unlock, delete and (from D3) a Protected intent
//! proof — counts on the host's one per-identity [`AttemptLimiter`]; a
//! locked identity is refused before Argon2id runs.

use std::sync::Arc;

use rekindle_db::paths::DataRoot;
use rekindle_db::registry::{self, IdentityEntry};
use rekindle_db::{Db, DbHandle, SqliteFriendStore};
use rekindle_idempotency::attempts::AttemptLimiter;
use rekindle_vault::{VaultHandle, VaultKey, VaultStore};
use zeroize::Zeroizing;

/// The host's slots for the unlocked identity.
pub struct Stores<'a> {
    pub db: &'a Arc<DbHandle>,
    pub vault: &'a VaultHandle,
    pub attempts: &'a AttemptLimiter<String>,
}

/// What an unlock gives the host besides the installed vault and database.
pub struct Unlocked {
    pub public_key: String,
    pub display_name: String,
    /// The Ed25519 identity secret.
    pub secret: Zeroizing<[u8; 32]>,
    /// The receive path's friend authority over the installed database.
    pub friend_store: Arc<SqliteFriendStore>,
    /// Wrong passphrases this unlock cleared (the host records them in
    /// the identity's audit chain).
    pub failed_attempts: u32,
}

/// An identity operation failed.
#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("no such identity")]
    NotFound,
    #[error("this identity is already on this device")]
    Exists,
    #[error("wrong passphrase")]
    WrongPassphrase,
    #[error("too many wrong passphrases; try again in {retry_after_ms} ms")]
    Locked { retry_after_ms: u64 },
    #[error("display name: {0}")]
    DisplayName(String),
    #[error("the identity is damaged: {0}")]
    Corrupt(String),
    #[error(transparent)]
    Vault(#[from] rekindle_vault::VaultError),
    #[error(transparent)]
    Db(#[from] rekindle_db::DbError),
    #[error(transparent)]
    Registry(#[from] rekindle_db::registry::RegistryError),
    #[error("identity files: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Storage(String),
}

/// The identities this root holds, oldest first.
///
/// # Errors
/// The registry cannot be read.
pub fn list(root: &DataRoot) -> Result<Vec<IdentityEntry>, IdentityError> {
    Ok(registry::load(&root.registry())?.identities)
}

fn map_open(e: rekindle_vault::VaultError) -> IdentityError {
    match e {
        rekindle_vault::VaultError::WrongPassphrase => IdentityError::WrongPassphrase,
        rekindle_vault::VaultError::NotFound | rekindle_vault::VaultError::HeaderMissing => {
            IdentityError::NotFound
        }
        other => IdentityError::Vault(other),
    }
}

/// An opened vault, its database key, and the wrong passphrases the open
/// cleared.
struct Opened {
    vault: VaultStore,
    app_db_key: rekindle_vault::AppDbKey,
    cleared_failures: u32,
}

/// Open `public_key`'s vault, counting a wrong passphrase.
fn open_vault(
    root: &DataRoot,
    attempts: &AttemptLimiter<String>,
    public_key: &str,
    passphrase: &str,
    now_ms: u64,
) -> Result<Opened, IdentityError> {
    let key = public_key.to_string();
    if let Err(retry_after_ms) = attempts.check(&key, now_ms) {
        return Err(IdentityError::Locked { retry_after_ms });
    }
    match VaultStore::open(&root.identity_vault(public_key), passphrase) {
        Ok((vault, app_db_key)) => Ok(Opened {
            vault,
            app_db_key,
            cleared_failures: attempts.succeed(&key),
        }),
        Err(rekindle_vault::VaultError::WrongPassphrase) => {
            attempts.fail(&key, now_ms);
            Err(IdentityError::WrongPassphrase)
        }
        Err(e) => Err(map_open(e)),
    }
}

/// Check `passphrase` against `public_key`'s vault (Argon2id: call it off
/// the async executor). A wrong one counts against the identity.
///
/// # Errors
/// Not found, wrong or locked out.
pub fn verify_passphrase(
    root: &DataRoot,
    attempts: &AttemptLimiter<String>,
    public_key: &str,
    passphrase: &str,
    now_ms: u64,
) -> Result<(), IdentityError> {
    if registry::load(&root.registry())?.get(public_key).is_none() {
        return Err(IdentityError::NotFound);
    }
    open_vault(root, attempts, public_key, passphrase, now_ms).map(drop)
}

/// Whether `passphrase` opens `public_key`'s vault, counting nothing: the
/// intent gate counts a wrong proof on the shared limiter itself (plan
/// D3.3). Argon2id: call it off the async executor.
///
/// # Errors
/// Not found, or the vault cannot be read.
pub fn passphrase_matches(
    root: &DataRoot,
    public_key: &str,
    passphrase: &str,
) -> Result<bool, IdentityError> {
    if registry::load(&root.registry())?.get(public_key).is_none() {
        return Err(IdentityError::NotFound);
    }
    match VaultStore::open(&root.identity_vault(public_key), passphrase) {
        Ok(_) => Ok(true),
        Err(rekindle_vault::VaultError::WrongPassphrase) => Ok(false),
        Err(e) => Err(map_open(e)),
    }
}

/// Install the opened vault and database.
fn install(
    stores: &Stores<'_>,
    vault: VaultStore,
    db: Db,
    public_key: String,
    display_name: String,
    secret: Zeroizing<[u8; 32]>,
    failed_attempts: u32,
) -> Unlocked {
    *stores.vault.lock() = Some(vault);
    stores.db.set(db);
    Unlocked {
        public_key,
        display_name,
        secret,
        friend_store: Arc::new(SqliteFriendStore::new(Arc::clone(stores.db))),
        failed_attempts,
    }
}

/// Remove an identity's files: vault, header, database (+ WAL and SHM),
/// and its file cache. A rotation removes the old key's cache too: its
/// names are keyed by the old secret, so the new one cannot find them.
fn remove_files(root: &DataRoot, public_key: &str) -> Result<(), IdentityError> {
    VaultStore::delete_files(&root.identity_vault(public_key))?;
    let db = root.identity_db(public_key);
    for suffix in ["", "-wal", "-shm"] {
        let mut p = db.as_os_str().to_owned();
        p.push(suffix);
        match std::fs::remove_file(std::path::PathBuf::from(p)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    match std::fs::remove_dir_all(root.identity_files(public_key)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Create an identity and unlock it: vault → database → identity row →
/// registry. Any failure removes the files it wrote.
///
/// # Errors
/// A display name over its limit, or a store that could not be written.
pub async fn create(
    root: &DataRoot,
    stores: &Stores<'_>,
    display_name: Option<&str>,
    passphrase: &str,
    now_ms: i64,
) -> Result<Unlocked, IdentityError> {
    let identity = rekindle_crypto::Identity::generate();
    let public_key = identity.public_key_hex();
    let secret = Zeroizing::new(*identity.secret_key_bytes());
    let display_name = match display_name.filter(|s| !s.trim().is_empty()) {
        Some(name) => rekindle_types::presence::limits::display_name(name)
            .map_err(|e| IdentityError::DisplayName(e.to_string()))?
            .to_string(),
        None => format!("User_{}", rekindle_utils::text::prefix(&public_key, 8)),
    };
    let created = create_files(
        root,
        &public_key,
        &display_name,
        &secret,
        passphrase,
        now_ms,
    )
    .await;
    let (vault, db) = match created {
        Ok(v) => v,
        Err(e) => {
            let _ = remove_files(root, &public_key);
            return Err(e);
        }
    };
    Ok(install(
        stores,
        vault,
        db,
        public_key,
        display_name,
        secret,
        0,
    ))
}

async fn create_files(
    root: &DataRoot,
    public_key: &str,
    display_name: &str,
    secret: &[u8; 32],
    passphrase: &str,
    now_ms: i64,
) -> Result<(VaultStore, Db), IdentityError> {
    let (vault, app_db_key) = VaultStore::create(&root.identity_vault(public_key), passphrase)?;
    vault.put(&VaultKey::IdentityEd25519, secret)?;
    let db = rekindle_db::open(&root.identity_db(public_key), app_db_key.as_bytes())?;
    let (pk, dn) = (public_key.to_string(), display_name.to_string());
    db.call(move |conn| rekindle_db::repo::identity::insert(conn, &pk, &dn, now_ms))
        .await
        .map_err(|e| IdentityError::Storage(e.to_string()))?;
    let mut reg = registry::load(&root.registry())?;
    reg.upsert(IdentityEntry {
        public_key: public_key.to_string(),
        display_name: display_name.to_string(),
        created_at: now_ms,
    });
    registry::save(&root.registry(), &reg)?;
    Ok((vault, db))
}

/// Unlock `public_key`: registry → vault (counting a wrong passphrase) →
/// database → identity row.
///
/// # Errors
/// Not found, wrong or locked out, or a damaged identity.
pub async fn unlock(
    root: &DataRoot,
    stores: &Stores<'_>,
    public_key: &str,
    passphrase: &str,
    now_ms: u64,
) -> Result<Unlocked, IdentityError> {
    if registry::load(&root.registry())?.get(public_key).is_none() {
        return Err(IdentityError::NotFound);
    }
    let opened = open_vault(root, stores.attempts, public_key, passphrase, now_ms)?;
    // A rotation journalled before its notices went out (audit 2026-10-20
    // R-2): our friends may know only the new key, so the identity comes
    // up under it, the rotation completed from the journal.
    if let Some(pending) = rekindle_vault::typed::pending_rotation::load(&opened.vault)? {
        drop(opened);
        tracing::info!("completing the journalled identity rotation");
        let entry = rotate(root, stores, public_key, &pending, passphrase, now_ms).await?;
        let opened = open_vault(root, stores.attempts, &entry.public_key, passphrase, now_ms)?;
        return unlock_opened(root, stores, &entry.public_key, opened).await;
    }
    unlock_opened(root, stores, public_key, opened).await
}

/// Journal our rotation (`pending`) in the unlocked identity's vault,
/// before its prekey set is published and before any friend is told.
///
/// # Errors
/// No identity is unlocked, or the vault write failed.
pub fn journal_rotation(
    stores: &Stores<'_>,
    pending: &rekindle_vault::typed::pending_rotation::PendingRotation,
) -> Result<(), IdentityError> {
    let guard = stores.vault.lock();
    let vault = guard
        .as_ref()
        .ok_or_else(|| IdentityError::Storage("no identity is unlocked".into()))?;
    rekindle_vault::typed::pending_rotation::persist(vault, pending)?;
    Ok(())
}

/// Drop the unlocked identity's rotation journal: the rotation stopped
/// before any friend was told.
///
/// # Errors
/// No identity is unlocked, or the vault write failed.
pub fn abandon_rotation(stores: &Stores<'_>) -> Result<(), IdentityError> {
    let guard = stores.vault.lock();
    let vault = guard
        .as_ref()
        .ok_or_else(|| IdentityError::Storage("no identity is unlocked".into()))?;
    rekindle_vault::typed::pending_rotation::clear(vault)?;
    Ok(())
}

/// The rest of [`unlock`], over the opened vault of `public_key`.
async fn unlock_opened(
    root: &DataRoot,
    stores: &Stores<'_>,
    public_key: &str,
    opened: Opened,
) -> Result<Unlocked, IdentityError> {
    let entry = registry::load(&root.registry())?
        .get(public_key)
        .cloned()
        .ok_or(IdentityError::NotFound)?;
    let Opened {
        vault,
        app_db_key,
        cleared_failures,
    } = opened;
    let stored = vault
        .get(&VaultKey::IdentityEd25519)?
        .ok_or_else(|| IdentityError::Corrupt("no private key in the vault".into()))?;
    let secret: [u8; 32] = stored
        .as_slice()
        .try_into()
        .map_err(|_| IdentityError::Corrupt("private key is not 32 bytes".into()))?;
    let secret = Zeroizing::new(secret);
    if rekindle_crypto::Identity::from_secret_bytes(&secret).public_key_hex() != public_key {
        return Err(IdentityError::Corrupt(
            "the vault's key does not match the identity".into(),
        ));
    }
    let db = rekindle_db::open(&root.identity_db(public_key), app_db_key.as_bytes())?;
    let pk = public_key.to_string();
    let display_name = db
        .call(move |conn| rekindle_db::repo::identity::login_row(conn, &pk))
        .await
        .map_err(|e| IdentityError::Storage(e.to_string()))?
        .map_or(entry.display_name, |(name, _)| name);
    Ok(install(
        stores,
        vault,
        db,
        public_key.to_string(),
        display_name,
        secret,
        cleared_failures,
    ))
}

/// Record a new display name in the registry, so the login picker shows
/// it (the identity row holds the same name).
///
/// # Errors
/// Not found, or the registry cannot be written.
pub fn rename(root: &DataRoot, public_key: &str, display_name: &str) -> Result<(), IdentityError> {
    let mut reg = registry::load(&root.registry())?;
    let entry = reg
        .identities
        .iter_mut()
        .find(|e| e.public_key == public_key)
        .ok_or(IdentityError::NotFound)?;
    entry.display_name = display_name.to_string();
    registry::save(&root.registry(), &reg)?;
    Ok(())
}

/// Take the unlocked identity's vault and database out of the host
/// (every task using them has stopped). The database closes when its last
/// handle drops.
pub async fn lock(stores: &Stores<'_>) {
    if let Err(e) = stores.db.clear().await {
        tracing::warn!(error = %e, "identity database still held at lock");
    }
    stores.vault.lock().take();
}

/// Delete an identity: its passphrase is checked first, then its files and
/// registry entry go. The caller has locked it if it was unlocked.
///
/// # Errors
/// Not found, wrong or locked out, or a file that cannot be removed.
pub fn delete(
    root: &DataRoot,
    attempts: &AttemptLimiter<String>,
    public_key: &str,
    passphrase: &str,
    now_ms: u64,
) -> Result<(), IdentityError> {
    verify_passphrase(root, attempts, public_key, passphrase, now_ms)?;
    remove(root, public_key)
}

/// Remove an identity's files and registry entry without a passphrase:
/// the daemon's typed-phrase delete until D3 makes the passphrase its
/// Protected proof. The caller has locked it if it was unlocked.
///
/// # Errors
/// Not found, or a file that cannot be removed.
pub fn remove(root: &DataRoot, public_key: &str) -> Result<(), IdentityError> {
    let mut reg = registry::load(&root.registry())?;
    if !reg.remove(public_key) {
        return Err(IdentityError::NotFound);
    }
    remove_files(root, public_key)?;
    registry::save(&root.registry(), &reg)?;
    Ok(())
}

mod archive;
mod rotate;
mod transfer;

pub use rotate::rotate;
pub use transfer::{export, import};

#[cfg(test)]
mod tests;

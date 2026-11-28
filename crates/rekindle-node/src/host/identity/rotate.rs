//! Rotating an identity's Ed25519 key (plan D1): its files are named by its
//! public key, so a rotation moves them.
//!
//! The rotated identity is built beside the old one: copies of the vault,
//! header and database under the new key, the copy vault holding the new
//! secret and the copy database re-keyed to it. The registry save is the
//! switch-over: before it the old identity is untouched and the copies are
//! removed on failure; after it the old files are removed. The vault's
//! header (salt and parameters) moves unchanged, so the passphrase and the
//! database key stay the same.
//!
//! The rotation is journalled (`rekindle_vault::typed::pending_rotation`,
//! audit 2026-10-20 R-2) in the old vault before any friend hears of it;
//! the copy vault drops the journal, and the old vault goes with it after
//! the registry save. [`super::unlock`] resumes a journalled rotation.

use rekindle_db::paths::DataRoot;
use rekindle_db::registry::{self, IdentityEntry};
use rekindle_vault::typed::pending_rotation::{self, PendingRotation};
use rekindle_vault::{VaultKey, VaultStore};

use super::{open_vault, remove_files, IdentityError, Stores};

/// Rotate `public_key` to the journalled identity `pending`. The identity
/// is locked (the caller locked it after telling its friends, which needs
/// the old key, or the unlock found the journal); `passphrase` opens the
/// copies, and a wrong one counts against the identity. The new
/// identity's set, already published to the same record (plan E2.4), is
/// re-minted from the journal and its rows are stored under the new key in
/// the re-key's transaction.
///
/// # Errors
/// Not found, wrong or locked out, the new key already here, or a store
/// that could not be copied or written.
pub async fn rotate(
    root: &DataRoot,
    stores: &Stores<'_>,
    public_key: &str,
    pending: &PendingRotation,
    passphrase: &str,
    now_ms: u64,
) -> Result<IdentityEntry, IdentityError> {
    let new_secret = &*pending.secret;
    let mut reg = registry::load(&root.registry())?;
    let old = reg
        .get(public_key)
        .cloned()
        .ok_or(IdentityError::NotFound)?;
    let new_identity = rekindle_crypto::Identity::from_secret_bytes(new_secret);
    let new_key = new_identity.public_key_hex();
    if reg.get(&new_key).is_some() {
        return Err(IdentityError::Exists);
    }
    // A resumed rotation finds the copies the interrupted one left: they
    // are rebuilt from the old files.
    if root.identity_vault(&new_key).exists() || root.identity_db(&new_key).exists() {
        remove_files(root, &new_key)?;
    }
    let prekeys = rekindle_transport::operations::identity::mint_rotation_set(
        &new_identity,
        &pending.mint_seed,
        pending.issued_at_ms,
    );
    drop(open_vault(
        root,
        stores.attempts,
        public_key,
        passphrase,
        now_ms,
    )?);

    let copied = build_copy(root, public_key, &new_key, new_secret, passphrase, prekeys).await;
    if let Err(e) = copied {
        let _ = remove_files(root, &new_key);
        return Err(e);
    }

    let entry = IdentityEntry {
        public_key: new_key.clone(),
        ..old
    };
    reg.remove(public_key);
    reg.upsert(entry.clone());
    if let Err(e) = registry::save(&root.registry(), &reg) {
        let _ = remove_files(root, &new_key);
        return Err(e.into());
    }
    if let Err(e) = remove_files(root, public_key) {
        tracing::warn!(error = %e, "the rotated identity's old files remain");
    }
    stores.attempts.succeed(&public_key.to_string());
    Ok(entry)
}

/// Copy `old`'s files under `new`, then put the new secret in the copy
/// vault (which drops the journal it copied) and re-key the copy database.
async fn build_copy(
    root: &DataRoot,
    old: &str,
    new: &str,
    new_secret: &[u8; 32],
    passphrase: &str,
    prekeys: rekindle_crypto::signal::prekey_record::MintedSet,
) -> Result<(), IdentityError> {
    let old_vault = root.identity_vault(old);
    let vault_bytes = std::fs::read(&old_vault)?;
    let header_bytes = std::fs::read(rekindle_vault::header::header_path(&old_vault))?;
    VaultStore::import_files(&root.identity_vault(new), &vault_bytes, &header_bytes)?;
    copy_database(root, old, new)?;

    let (vault, app_db_key) = VaultStore::open(&root.identity_vault(new), passphrase)?;
    vault.put(&VaultKey::IdentityEd25519, new_secret)?;
    pending_rotation::clear(&vault)?;
    drop(vault);
    let db = rekindle_db::open(&root.identity_db(new), app_db_key.as_bytes())?;
    let (old_key, new_key) = (old.to_string(), new.to_string());
    db.call(move |conn| {
        rekindle_db::repo::identity::rekey(conn, &old_key, &new_key, Some(&prekeys))
    })
    .await
    .map_err(|e| IdentityError::Storage(e.to_string()))?;
    if let Err(e) = db.close().await {
        tracing::warn!(error = %e, "closing the rotated database failed");
    }
    Ok(())
}

/// Copy the database file and, when one is left, its WAL (a closed
/// database has none: the last close checkpoints it).
fn copy_database(root: &DataRoot, old: &str, new: &str) -> Result<(), IdentityError> {
    let (from, to) = (root.identity_db(old), root.identity_db(new));
    if to.exists() {
        return Err(IdentityError::Exists);
    }
    std::fs::copy(&from, &to)?;
    let mut from_wal = from.into_os_string();
    from_wal.push("-wal");
    let from_wal = std::path::PathBuf::from(from_wal);
    if from_wal.exists() {
        let mut to_wal = to.into_os_string();
        to_wal.push("-wal");
        std::fs::copy(&from_wal, std::path::PathBuf::from(to_wal))?;
    }
    Ok(())
}

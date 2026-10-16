//! Moving an identity between roots: [`export`] and [`import`] (plan D1).

use rekindle_db::paths::DataRoot;
use rekindle_db::registry::{self, IdentityEntry};
use rekindle_idempotency::attempts::AttemptLimiter;
use rekindle_vault::{VaultKey, VaultStore};

use super::{archive, open_vault, remove_files, IdentityError, Stores};

/// Export `public_key` as an archive (its registry entry, vault and
/// header), after checking `passphrase` (Argon2id: call it off the async
/// executor; a wrong one counts). The unlocked vault is read under its own
/// lock when it is this identity's, so no write interleaves.
///
/// # Errors
/// Not found, wrong or locked out, or the files cannot be read.
pub fn export(
    root: &DataRoot,
    stores: &Stores<'_>,
    public_key: &str,
    passphrase: &str,
    now_ms: u64,
) -> Result<Vec<u8>, IdentityError> {
    let entry = registry::load(&root.registry())?
        .get(public_key)
        .cloned()
        .ok_or(IdentityError::NotFound)?;
    let (opened, _) = open_vault(root, stores.attempts, public_key, passphrase, now_ms)?;
    let live = stores.vault.lock();
    let (vault, header) = match live.as_ref() {
        Some(unlocked) if unlocked.path() == opened.path() => unlocked.export_files()?,
        _ => opened.export_files()?,
    };
    drop(live);
    archive::Archive {
        entry,
        header,
        vault,
    }
    .encode()
}

/// Install an identity from an archive: its vault and header are written,
/// opened with `passphrase` (a wrong one counts against the identity) and
/// checked against the archive's public key; then a new database, its
/// identity row and the registry entry. Nothing is unlocked. Any failure
/// removes what it wrote.
///
/// # Errors
/// A damaged archive, an identity already here, wrong or locked out, or a
/// store that could not be written.
pub async fn import(
    root: &DataRoot,
    attempts: &AttemptLimiter<String>,
    archive_bytes: &[u8],
    passphrase: &str,
    now_ms: u64,
) -> Result<IdentityEntry, IdentityError> {
    let archive = archive::Archive::decode(archive_bytes)?;
    let entry = archive.entry;
    // The key names files: it must be a key, not a path.
    rekindle_types::key_format::public_key_hex(&entry.public_key)
        .map_err(|e| IdentityError::Corrupt(format!("identity archive key: {e}")))?;
    let public_key = entry.public_key.clone();
    if registry::load(&root.registry())?.get(&public_key).is_some() {
        return Err(IdentityError::Exists);
    }
    if let Err(retry_after_ms) = attempts.check(&public_key, now_ms) {
        return Err(IdentityError::Locked { retry_after_ms });
    }
    VaultStore::import_files(
        &root.identity_vault(&public_key),
        &archive.vault,
        &archive.header,
    )?;
    let installed = install_imported(root, attempts, &entry, passphrase, now_ms).await;
    if installed.is_err() {
        let _ = remove_files(root, &public_key);
    }
    installed.map(|()| entry)
}

async fn install_imported(
    root: &DataRoot,
    attempts: &AttemptLimiter<String>,
    entry: &IdentityEntry,
    passphrase: &str,
    now_ms: u64,
) -> Result<(), IdentityError> {
    let public_key = &entry.public_key;
    let (vault, app_db_key) = open_vault(root, attempts, public_key, passphrase, now_ms)?;
    let stored = vault
        .get(&VaultKey::IdentityEd25519)?
        .ok_or_else(|| IdentityError::Corrupt("no private key in the vault".into()))?;
    let secret: [u8; 32] = stored
        .as_slice()
        .try_into()
        .map_err(|_| IdentityError::Corrupt("private key is not 32 bytes".into()))?;
    if rekindle_crypto::Identity::from_secret_bytes(&secret).public_key_hex() != *public_key {
        return Err(IdentityError::Corrupt(
            "the vault's key does not match the archive".into(),
        ));
    }
    let db = rekindle_db::open(&root.identity_db(public_key), app_db_key.as_bytes())?;
    let (pk, dn, at) = (
        public_key.clone(),
        entry.display_name.clone(),
        entry.created_at,
    );
    db.call(move |conn| rekindle_db::repo::identity::insert(conn, &pk, &dn, at))
        .await
        .map_err(|e| IdentityError::Storage(e.to_string()))?;
    if let Err(e) = db.close().await {
        tracing::warn!(error = %e, "closing the imported identity's database failed");
    }
    let mut reg = registry::load(&root.registry())?;
    reg.upsert(entry.clone());
    registry::save(&root.registry(), &reg)?;
    Ok(())
}

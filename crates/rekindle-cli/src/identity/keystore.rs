//! OS keyring integration for secret material.
//!
//! Stores Ed25519 signing keys and DHT record keypairs in the platform
//! keyring (Keychain on macOS, libsecret on Linux, Credential Manager
//! on Windows). All keyring operations are synchronous, so they're
//! wrapped in `tokio::task::spawn_blocking`.

use anyhow::Context;

const SERVICE: &str = "rekindle";
const KEY_SIGNING: &str = "identity-signing-key";
const KEY_PREFIX_KEYPAIR: &str = "keypair-";

/// Store the Ed25519 signing key in the OS keyring.
///
/// The key is stored as a hex-encoded string. The caller MUST zeroize
/// the source bytes immediately after this call returns.
pub async fn store_signing_key(key_bytes: &[u8; 32]) -> anyhow::Result<()> {
    let hex = hex::encode(key_bytes);
    store_keyring_entry(KEY_SIGNING, &hex)
        .await
        .context("failed to store signing key in keyring")
}

/// Load the Ed25519 signing key from the OS keyring.
///
/// Returns the raw 32-byte key. The caller MUST wrap in `Zeroizing<_>`.
pub async fn load_signing_key() -> anyhow::Result<[u8; 32]> {
    let hex_str = load_keyring_entry(KEY_SIGNING)
        .await
        .context("failed to load signing key from keyring")?
        .ok_or_else(|| {
            crate::error::CliError::NotInitialized(
                "signing key not found in keyring — run: rekindle init".into(),
            )
        })?;

    let bytes = hex::decode(&hex_str)
        .context("signing key in keyring is not valid hex")?;

    if bytes.len() != 32 {
        anyhow::bail!(
            "signing key in keyring has wrong length ({} bytes, expected 32)",
            bytes.len()
        );
    }

    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

/// Store a DHT record keypair in the OS keyring.
///
/// `label` identifies which keypair (e.g., "profile", "friend_list").
pub async fn store_keypair_bytes(label: &str, bytes: &[u8]) -> anyhow::Result<()> {
    let key = format!("{KEY_PREFIX_KEYPAIR}{label}");
    let hex = hex::encode(bytes);
    store_keyring_entry(&key, &hex)
        .await
        .with_context(|| format!("failed to store {label} keypair in keyring"))
}

/// Load a DHT record keypair from the OS keyring.
pub async fn load_keypair_bytes(label: &str) -> anyhow::Result<Option<Vec<u8>>> {
    let key = format!("{KEY_PREFIX_KEYPAIR}{label}");
    let hex_str = load_keyring_entry(&key).await?;
    match hex_str {
        Some(h) => {
            let bytes = hex::decode(&h)
                .with_context(|| format!("{label} keypair in keyring is not valid hex"))?;
            Ok(Some(bytes))
        }
        None => Ok(None),
    }
}

/// Delete all rekindle keyring entries.
///
/// Used by `rekindle identity destroy` and `rekindle init --wipe-all-data`.
/// Best-effort — continues on individual deletion failures.
pub async fn delete_all_keys() -> anyhow::Result<()> {
    let keys = [
        KEY_SIGNING.to_string(),
        format!("{KEY_PREFIX_KEYPAIR}profile"),
        format!("{KEY_PREFIX_KEYPAIR}friend_list"),
    ];

    for key in &keys {
        if let Err(e) = delete_keyring_entry(key).await {
            tracing::warn!(key, error = %e, "failed to delete keyring entry (may not exist)");
        }
    }

    Ok(())
}

// ── Keyring primitives (spawn_blocking wrappers) ────────────────────────

async fn store_keyring_entry(key: &str, value: &str) -> anyhow::Result<()> {
    let service = SERVICE.to_string();
    let key = key.to_string();
    let value = value.to_string();

    tokio::task::spawn_blocking(move || {
        let entry = keyring::Entry::new(&service, &key)
            .context("failed to create keyring entry")?;
        entry
            .set_password(&value)
            .context("failed to write to keyring")?;
        Ok(())
    })
    .await
    .context("keyring task panicked")?
}

async fn load_keyring_entry(key: &str) -> anyhow::Result<Option<String>> {
    let service = SERVICE.to_string();
    let key = key.to_string();

    tokio::task::spawn_blocking(move || {
        let entry = keyring::Entry::new(&service, &key)
            .context("failed to create keyring entry")?;
        match entry.get_password() {
            Ok(val) => Ok(Some(val)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(anyhow::anyhow!("keyring read failed: {e}")),
        }
    })
    .await
    .context("keyring task panicked")?
}

async fn delete_keyring_entry(key: &str) -> anyhow::Result<()> {
    let service = SERVICE.to_string();
    let key = key.to_string();

    tokio::task::spawn_blocking(move || {
        let entry = keyring::Entry::new(&service, &key)
            .context("failed to create keyring entry")?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()), // already deleted
            Err(e) => Err(anyhow::anyhow!("keyring delete failed: {e}")),
        }
    })
    .await
    .context("keyring task panicked")?
}

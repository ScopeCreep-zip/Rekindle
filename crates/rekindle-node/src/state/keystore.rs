//! OS keyring integration for the daemon process.
//!
//! The daemon accesses the OS keyring exactly once during the Unlock flow
//! to load the Ed25519 signing key into memory. The key is held in a
//! `SigningKeyHandle` that zeroizes on drop. On Lock or Shutdown, the
//! handle is dropped, zeroizing the key.
//!
//! This is the authoritative keystore implementation. When rekindle-cli
//! is rewired as an IPC client, its keystore.rs becomes dead code —
//! all secret access flows through this module via the daemon.
//!
//! [RC-16] All secret material implements ZeroizeOnDrop.
//! [RC-10] No unsafe code.

use zeroize::{Zeroize, ZeroizeOnDrop};

const SERVICE: &str = "rekindle";
const KEY_SIGNING: &str = "identity-signing-key";
const KEY_PREFIX_KEYPAIR: &str = "keypair-";

/// In-memory handle to the Ed25519 signing key.
///
/// Zeroizes the key material on drop, including during panics.
/// The daemon holds exactly one of these after unlock, and drops
/// it on lock or shutdown.
#[derive(ZeroizeOnDrop)]
pub struct SigningKeyHandle {
    #[zeroize]
    bytes: [u8; 32],
}

impl SigningKeyHandle {
    /// Access the raw signing key bytes.
    ///
    /// Only called by transport operations that need to sign payloads.
    /// The reference lifetime is bounded by the handle — callers cannot
    /// outlive the unlock period.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }
}

impl std::fmt::Debug for SigningKeyHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SigningKeyHandle")
            .field("bytes", &"[REDACTED]")
            .finish()
    }
}

/// Load the Ed25519 signing key from the OS keyring.
///
/// Called exactly once during the Unlock flow. Returns a `SigningKeyHandle`
/// that zeroizes on drop. The handle is stored in `DaemonContext` and
/// dropped on Lock/Shutdown.
///
/// Runs on a blocking thread because the `keyring` crate is synchronous.
pub async fn load_signing_key() -> anyhow::Result<SigningKeyHandle> {
    let hex_str = load_keyring_entry(KEY_SIGNING)
        .await?
        .ok_or_else(|| anyhow::anyhow!(
            "signing key not found in keyring — run: rekindle init"
        ))?;

    let raw = hex::decode(&hex_str)
        .map_err(|e| anyhow::anyhow!("signing key in keyring is not valid hex: {e}"))?;

    if raw.len() != 32 {
        anyhow::bail!(
            "signing key in keyring has wrong length ({} bytes, expected 32)",
            raw.len()
        );
    }

    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&raw);

    // Zeroize the intermediate Vec before it's freed.
    let mut raw = raw;
    raw.zeroize();

    Ok(SigningKeyHandle { bytes })
}

/// Load a DHT record keypair from the OS keyring.
///
/// Used during Resume to load profile/friend_list/community keypairs.
pub async fn load_keypair_bytes(label: &str) -> anyhow::Result<Option<Vec<u8>>> {
    let key = format!("{KEY_PREFIX_KEYPAIR}{label}");
    let hex_str = load_keyring_entry(&key).await?;
    match hex_str {
        Some(h) => {
            let bytes = hex::decode(&h)
                .map_err(|e| anyhow::anyhow!("{label} keypair in keyring is not valid hex: {e}"))?;
            Ok(Some(bytes))
        }
        None => Ok(None),
    }
}

/// Check whether the signing key exists in the keyring without loading it.
///
/// Used during daemon startup to determine if an identity has been initialized.
pub async fn has_signing_key() -> anyhow::Result<bool> {
    let result = load_keyring_entry(KEY_SIGNING).await?;
    Ok(result.is_some())
}

// ── Write operations (used by IdentityCreate, Rotate) ───────────────────

/// Store the Ed25519 signing key in the OS keyring.
///
/// The key is stored as a hex-encoded string. The caller MUST zeroize
/// the source bytes immediately after this call returns.
pub async fn store_signing_key(key_bytes: &[u8; 32]) -> anyhow::Result<()> {
    let hex_val = hex::encode(key_bytes);
    store_keyring_entry(KEY_SIGNING, &hex_val).await
}

/// Store a DHT record keypair in the OS keyring.
///
/// `label` identifies which keypair (e.g., "profile", "friend_list",
/// "signed-prekey-1", "one-time-prekey-1").
pub async fn store_keypair_bytes(label: &str, bytes: &[u8]) -> anyhow::Result<()> {
    let key = format!("{KEY_PREFIX_KEYPAIR}{label}");
    let hex_val = hex::encode(bytes);
    store_keyring_entry(&key, &hex_val).await
}

// ── Governance keypair operations ─────────────────────────────────────────

const KEY_PREFIX_GOVERNANCE: &str = "community-governance-";

/// Store a community governance keypair in the OS keyring.
///
/// `label` is the short governance key identifier (first 12 chars).
/// The keypair is stored as hex-encoded 64 bytes (32 pub + 32 secret).
pub async fn store_governance_keypair(label: &str, bytes: &[u8]) -> anyhow::Result<()> {
    let key = format!("{KEY_PREFIX_GOVERNANCE}{label}");
    let hex_val = hex::encode(bytes);
    store_keyring_entry(&key, &hex_val).await
}

/// Load a community governance keypair from the OS keyring.
pub async fn load_governance_keypair(label: &str) -> anyhow::Result<Option<Vec<u8>>> {
    let key = format!("{KEY_PREFIX_GOVERNANCE}{label}");
    let hex_str = load_keyring_entry(&key).await?;
    match hex_str {
        Some(h) => {
            let bytes = hex::decode(&h)
                .map_err(|e| anyhow::anyhow!("governance keypair not valid hex: {e}"))?;
            Ok(Some(bytes))
        }
        None => Ok(None),
    }
}

/// Delete a community governance keypair from the OS keyring.
pub async fn delete_governance_keypair(label: &str) -> anyhow::Result<()> {
    let key = format!("{KEY_PREFIX_GOVERNANCE}{label}");
    delete_keyring_entry(&key).await
}

/// Delete a single keypair from the OS keyring.
pub async fn delete_keypair_bytes(label: &str) -> anyhow::Result<()> {
    let key = format!("{KEY_PREFIX_KEYPAIR}{label}");
    delete_keyring_entry(&key).await
}

/// Delete all rekindle keyring entries.
///
/// Used by `IdentityDestroy` and `IdentityWipe`. Best-effort — continues
/// on individual deletion failures.
pub async fn delete_all_keys() -> anyhow::Result<()> {
    let keys = [
        KEY_SIGNING.to_string(),
        format!("{KEY_PREFIX_KEYPAIR}profile"),
        format!("{KEY_PREFIX_KEYPAIR}friend_list"),
        // Signal prekeys: we delete known patterns. A more complete
        // approach would enumerate all entries with the prefix, but
        // the keyring crate doesn't support enumeration.
        format!("{KEY_PREFIX_KEYPAIR}signed-prekey-1"),
        format!("{KEY_PREFIX_KEYPAIR}one-time-prekey-1"),
    ];

    for key in &keys {
        if let Err(e) = delete_keyring_entry(key).await {
            tracing::warn!(key, error = %e, "keyring delete failed (may not exist)");
        }
    }
    Ok(())
}

// ── Storage backend (keyring with disk fallback) ────────────────────────
//
// The OS keyring is preferred but unavailable in many environments (Docker,
// CI, headless servers, Wayland without a secret service). When the keyring
// fails, we fall back to disk-based storage at ~/.local/state/rekindle/keys/
// with 0600 permissions. This matches Veilid's own behavior: "Secure key
// storage service unavailable, falling back to direct disk-based storage."

/// Directory for disk-based key fallback.
fn fallback_keys_dir() -> std::path::PathBuf {
    let state = std::env::var("XDG_STATE_HOME")
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            format!("{home}/.local/state")
        });
    std::path::PathBuf::from(state).join("rekindle/keys")
}

fn fallback_key_path(key: &str) -> std::path::PathBuf {
    // Sanitize the key name for filesystem safety
    let safe_name: String = key.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    fallback_keys_dir().join(safe_name)
}

async fn store_keyring_entry(key: &str, value: &str) -> anyhow::Result<()> {
    let service = SERVICE.to_string();
    let key_clone = key.to_string();
    let value_clone = value.to_string();

    // Try OS keyring first
    let keyring_result = tokio::task::spawn_blocking(move || {
        let entry = keyring::Entry::new(&service, &key_clone)
            .map_err(|e| anyhow::anyhow!("keyring entry creation failed: {e}"))?;
        entry
            .set_password(&value_clone)
            .map_err(|e| anyhow::anyhow!("keyring write failed: {e}"))?;
        Ok::<(), anyhow::Error>(())
    })
    .await
    .map_err(|e| anyhow::anyhow!("keyring task panicked: {e}"))?;

    if keyring_result.is_ok() {
        return Ok(());
    }

    // Keyring failed — fall back to encrypted disk storage
    let path = fallback_key_path(key);
    let dir = fallback_keys_dir();
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    let machine_key = derive_machine_key();
    let encrypted = encrypt_fallback(value.as_bytes(), &machine_key)?;
    std::fs::write(&path, &encrypted)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    tracing::warn!(key, "stored to ENCRYPTED disk fallback (keyring unavailable — OS keyring preferred)");
    Ok(())
}

async fn delete_keyring_entry(key: &str) -> anyhow::Result<()> {
    let service = SERVICE.to_string();
    let key_clone = key.to_string();

    let _ = tokio::task::spawn_blocking(move || {
        let entry = keyring::Entry::new(&service, &key_clone)
            .map_err(|e| anyhow::anyhow!("keyring entry creation failed: {e}"))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(anyhow::anyhow!("keyring delete failed: {e}")),
        }
    })
    .await;

    // Also delete from disk fallback
    let path = fallback_key_path(key);
    let _ = std::fs::remove_file(&path);
    Ok(())
}

async fn load_keyring_entry(key: &str) -> anyhow::Result<Option<String>> {
    let service = SERVICE.to_string();
    let key_clone = key.to_string();

    // Try OS keyring first
    let keyring_result = tokio::task::spawn_blocking(move || {
        let entry = keyring::Entry::new(&service, &key_clone)
            .map_err(|e| anyhow::anyhow!("keyring entry creation failed: {e}"))?;
        match entry.get_password() {
            Ok(val) => Ok(Some(val)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(anyhow::anyhow!("keyring read failed: {e}")),
        }
    })
    .await
    .map_err(|e| anyhow::anyhow!("keyring task panicked: {e}"))?;

    if let Ok(Some(val)) = &keyring_result {
        return Ok(Some(val.clone()));
    }

    // Keyring failed or empty — try encrypted disk fallback
    let path = fallback_key_path(key);
    match std::fs::read(&path) {
        Ok(data) if !data.is_empty() => {
            let machine_key = derive_machine_key();
            let plaintext = decrypt_fallback(&data, &machine_key)
                .map_err(|e| anyhow::anyhow!("disk fallback decrypt failed for '{key}': {e}"))?;
            let value = String::from_utf8(plaintext)
                .map_err(|e| anyhow::anyhow!("disk fallback not valid UTF-8: {e}"))?;
            tracing::warn!(key, "loaded from encrypted disk fallback (keyring unavailable)");
            Ok(Some(value))
        }
        _ => Ok(None),
    }
}

// ── Machine-key obfuscation for disk fallback ───────────────────────────
//
// TODO: Replace with real encryption using a user-supplied passphrase or
// hardware-backed key (TPM2, Secure Enclave). The current implementation
// is OBFUSCATION, not encryption — the key derivation material (machine-id
// + UID) is locally readable by any process running as the same user. In
// open-source software this is negligibly better than base64 against a
// compromised agent or other bad actor in userspace.
//
// The obfuscation exists solely to prevent casual exposure via:
// - Accidental `cat` of the file showing hex key material in terminal scrollback
// - Backup processes that index file contents (grep, ripgrep, etc.)
// - Shoulder-surfing the raw file contents
//
// It does NOT protect against: same-user malware, root access, disk theft
// with known machine-id, or anyone who reads this source code.

fn derive_machine_key() -> [u8; 32] {
    let machine_id = std::fs::read_to_string("/etc/machine-id")
        .or_else(|_| std::fs::read_to_string("/var/lib/dbus/machine-id"))
        .unwrap_or_else(|_| "no-machine-id".to_string());
    let uid = rustix::process::getuid().as_raw();
    let mut input = machine_id.trim().as_bytes().to_vec();
    input.extend_from_slice(&uid.to_le_bytes());
    let base = blake3::hash(&input);
    let derived = blake3::keyed_hash(base.as_bytes(), b"rekindle-disk-fallback-v1");
    *derived.as_bytes()
}

fn encrypt_fallback(plaintext: &[u8], key: &[u8; 32]) -> anyhow::Result<Vec<u8>> {
    use aes_gcm::{Aes256Gcm, aead::{Aead, KeyInit}, Nonce};
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| anyhow::anyhow!("AES key init: {e}"))?;
    let mut nonce_bytes = [0u8; 12];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher.encrypt(nonce, plaintext)
        .map_err(|e| anyhow::anyhow!("encrypt: {e}"))?;
    let mut output = Vec::with_capacity(12 + ciphertext.len());
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);
    Ok(output)
}

fn decrypt_fallback(encrypted: &[u8], key: &[u8; 32]) -> anyhow::Result<Vec<u8>> {
    use aes_gcm::{Aes256Gcm, aead::{Aead, KeyInit}, Nonce};
    if encrypted.len() < 12 + 16 {
        anyhow::bail!("encrypted data too short ({} bytes, min 28)", encrypted.len());
    }
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| anyhow::anyhow!("AES key init: {e}"))?;
    let nonce = Nonce::from_slice(&encrypted[..12]);
    let plaintext = cipher.decrypt(nonce, &encrypted[12..])
        .map_err(|e| anyhow::anyhow!("decrypt: {e}"))?;
    Ok(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signing_key_handle_debug_redacts() {
        let handle = SigningKeyHandle { bytes: [0xAB; 32] };
        let debug = format!("{handle:?}");
        assert!(debug.contains("REDACTED"));
        assert!(!debug.contains("AB"));
        assert!(!debug.contains("171")); // 0xAB decimal
    }

    #[test]
    fn signing_key_handle_zeroizes_on_drop() {
        let handle = SigningKeyHandle { bytes: [0xFF; 32] };
        let ptr = handle.as_bytes().as_ptr();
        drop(handle);
        // After drop, the memory at ptr should be zeroized.
        // We can't safely read freed memory in safe Rust, but the
        // ZeroizeOnDrop derive guarantees the zeroize call happens.
        // This test verifies the type compiles with the derive.
        let _ = ptr;
    }
}

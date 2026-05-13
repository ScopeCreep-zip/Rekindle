//! Key management and persistence for Noise IK keypairs.
//!
//! Handles generation, filesystem I/O, permissions, and tamper-detection
//! checksums. Adapted from open-sesame `core-ipc/src/noise_keys.rs`.
//!
//! Key storage layout under `$XDG_RUNTIME_DIR/rekindle/`:
//! - `bus.pub` (0644) — bus server public key, read by connecting clients
//! - `bus.key` (0600) — bus server private key (atomic write)
//! - `bus.checksum` — BLAKE3 integrity checksum
//! - `keys/<agent>.pub` (0644) — per-agent public key
//! - `keys/<agent>.key` (0600) — per-agent private key (atomic write)
//! - `keys/<agent>.checksum` — per-agent integrity checksum
//!
//! [RC-4] All private key writes use atomic tmp+fsync+rename pattern.
//! [RC-6] Private keys are created with mode 0600 from the moment of creation.
//! [RC-10] No unsafe code in this module.
//! [RC-16] Private key bytes wrapped in `ZeroizingKeypair` for drop cleanup.


use std::path::{Path, PathBuf};

use super::error::{IpcError, Result};
use super::transport::runtime_dir;

// ── Agent name validation ────────────────────────────���─────────────────

/// Validate that an agent name is safe for use in file paths.
///
/// Must match `[a-zA-Z0-9_-]+`. Rejects empty strings, paths with `/`,
/// `..`, NUL bytes, or any other character that could enable path traversal.
/// [RC-5] [RC-9]
pub fn validate_agent_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(IpcError::InvalidAgentName {
            name: "(empty)".into(),
        });
    }
    for ch in name.chars() {
        if !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-' {
            return Err(IpcError::InvalidAgentName {
                name: name.into(),
            });
        }
    }
    Ok(())
}

// ── BLAKE3 tamper detection ────────────────────────────────────��───────

/// Compute an integrity-detection checksum for a keypair.
///
/// Uses BLAKE3 keyed hash with the public key as the 32-byte key and
/// the private key as the data. Detects partial corruption or partial
/// tampering (private key replaced but checksum file untouched).
///
/// Does NOT prevent an attacker with full filesystem write access from
/// replacing all three files — that requires a root-of-trust outside
/// the filesystem (e.g., TPM-backed attestation).
fn keypair_checksum(public_key: &[u8; 32], private_key: &[u8]) -> [u8; 32] {
    *blake3::keyed_hash(public_key, private_key).as_bytes()
}

// ── ZeroizingKeypair ───────────────────────────────────────────────────

/// Zeroize-on-drop wrapper for `snow::Keypair`.
///
/// `snow::Keypair` has no `Drop` impl, so the private key persists in freed
/// memory if not explicitly zeroized. This wrapper guarantees zeroization
/// on drop, including during panics (unwind calls `Drop`). [RC-16]
pub struct ZeroizingKeypair {
    inner: snow::Keypair,
}

impl ZeroizingKeypair {
    /// Wrap a `snow::Keypair`, taking ownership.
    #[must_use]
    pub fn new(keypair: snow::Keypair) -> Self {
        Self { inner: keypair }
    }

    /// Access the public key (32 bytes).
    #[must_use]
    pub fn public(&self) -> &[u8] {
        &self.inner.public
    }

    /// Access the private key (32 bytes). Use only for `snow::Builder` calls.
    #[must_use]
    pub fn private(&self) -> &[u8] {
        &self.inner.private
    }

    /// Borrow the inner `snow::Keypair` for APIs that require `&snow::Keypair`.
    #[must_use]
    pub fn as_inner(&self) -> &snow::Keypair {
        &self.inner
    }

    /// Consume and return inner keypair. Caller takes zeroization responsibility.
    #[must_use]
    pub fn into_inner(mut self) -> snow::Keypair {
        let private = std::mem::take(&mut self.inner.private);
        let public = std::mem::take(&mut self.inner.public);
        snow::Keypair { private, public }
    }
}

impl Drop for ZeroizingKeypair {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.inner.private);
    }
}

impl From<snow::Keypair> for ZeroizingKeypair {
    fn from(keypair: snow::Keypair) -> Self {
        Self::new(keypair)
    }
}

// ── Key generation ─────────────────────────────────────────────────────

/// Noise IK protocol parameter string.
///
/// AES-256-GCM with SHA-256: uses AES-NI + CLMUL hardware acceleration
/// on x86-64 via the `ring` crate (`ring-accelerated` snow feature).
/// On platforms without AES-NI, ring falls back to a constant-time
/// software implementation (slower than ChaCha20, but still correct).
///
/// Previous: `Noise_IK_25519_ChaChaPoly_BLAKE2s` (pure-Rust, ~1.5μs/frame)
/// Current:  `Noise_IK_25519_AESGCM_SHA256` (AES-NI hardware, expected ~300-500ns/frame)
pub const NOISE_PARAMS: &str = "Noise_IK_25519_AESGCM_SHA256";

/// Generate a new X25519 static keypair for Noise IK.
pub fn generate_keypair() -> Result<ZeroizingKeypair> {
    let builder = snow::Builder::new(NOISE_PARAMS.parse().map_err(|e| {
        IpcError::HandshakeFailed {
            reason: format!("invalid Noise params: {e}"),
        }
    })?);
    let keypair = builder.generate_keypair().map_err(|e| {
        IpcError::HandshakeFailed {
            reason: format!("keypair generation failed: {e}"),
        }
    })?;
    Ok(ZeroizingKeypair::new(keypair))
}

// ── Key persistence ────────────────────────────────────────────────────

fn keys_dir() -> Result<PathBuf> {
    Ok(runtime_dir()?.join("keys"))
}

/// Create the per-agent keys directory if it does not exist.
pub async fn create_keys_dir() -> Result<()> {
    let dir = keys_dir()?;
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| IpcError::DirectoryCreate {
            path: dir.display().to_string(),
            source: e,
        })?;

    // [RC-6] Restrict to owner-only (0700).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .await
            .map_err(|e| IpcError::DirectoryCreate {
                path: dir.display().to_string(),
                source: e,
            })?;
    }
    Ok(())
}

/// Write the bus server's static keypair to the runtime directory.
pub async fn write_bus_keypair(keypair: &snow::Keypair) -> Result<()> {
    let dir = runtime_dir()?;
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| IpcError::DirectoryCreate {
            path: dir.display().to_string(),
            source: e,
        })?;

    let pub_path = dir.join("bus.pub");
    let key_path = dir.join("bus.key");

    // Public key: world-readable.
    tokio::fs::write(&pub_path, &keypair.public)
        .await
        .map_err(IpcError::Io)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&pub_path, std::fs::Permissions::from_mode(0o644))
            .await
            .map_err(IpcError::Io)?;
    }

    // [RC-4] Private key: atomic write with 0600 from creation.
    write_private_key_atomic(&key_path, &keypair.private).await?;

    // BLAKE3 tamper-detection checksum.
    let pub_array: [u8; 32] = keypair
        .public
        .clone()
        .try_into()
        .map_err(|_| IpcError::HandshakeFailed {
            reason: "bus public key is not 32 bytes".into(),
        })?;
    let checksum = keypair_checksum(&pub_array, &keypair.private);
    tokio::fs::write(dir.join("bus.checksum"), checksum)
        .await
        .map_err(IpcError::Io)?;

    tracing::info!(pub_path = %pub_path.display(), "bus keypair written");
    Ok(())
}

/// Write a per-agent keypair to disk.
pub async fn write_agent_keypair(agent_name: &str, keypair: &snow::Keypair) -> Result<()> {
    validate_agent_name(agent_name)?;
    let dir = keys_dir()?;

    let pub_path = dir.join(format!("{agent_name}.pub"));
    let key_path = dir.join(format!("{agent_name}.key"));

    tokio::fs::write(&pub_path, &keypair.public)
        .await
        .map_err(IpcError::Io)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&pub_path, std::fs::Permissions::from_mode(0o644))
            .await
            .map_err(IpcError::Io)?;
    }

    write_private_key_atomic(&key_path, &keypair.private).await?;

    // Checksum.
    let pub_array: [u8; 32] = keypair
        .public
        .clone()
        .try_into()
        .map_err(|_| IpcError::HandshakeFailed {
            reason: format!("{agent_name} public key is not 32 bytes"),
        })?;
    let checksum = keypair_checksum(&pub_array, &keypair.private);
    tokio::fs::write(dir.join(format!("{agent_name}.checksum")), checksum)
        .await
        .map_err(IpcError::Io)?;

    tracing::debug!(agent = agent_name, "agent keypair written");
    Ok(())
}

/// Read the bus server's public key.
pub async fn read_bus_public_key() -> Result<[u8; 32]> {
    let dir = runtime_dir()?;
    let pub_path = dir.join("bus.pub");
    let bytes = tokio::fs::read(&pub_path)
        .await
        .map_err(IpcError::Io)?;
    bytes.try_into().map_err(|v: Vec<u8>| {
        IpcError::HandshakeFailed {
            reason: format!("bus.pub: expected 32 bytes, got {}", v.len()),
        }
    })
}

/// Read an agent's keypair from disk with tamper detection.
pub async fn read_agent_keypair(
    agent_name: &str,
) -> Result<(zeroize::Zeroizing<Vec<u8>>, [u8; 32])> {
    validate_agent_name(agent_name)?;
    let dir = keys_dir()?;

    let key_path = dir.join(format!("{agent_name}.key"));
    let pub_path = dir.join(format!("{agent_name}.pub"));

    let private_bytes = tokio::fs::read(&key_path)
        .await
        .map_err(IpcError::Io)?;
    let public_bytes = tokio::fs::read(&pub_path)
        .await
        .map_err(IpcError::Io)?;

    let public_key: [u8; 32] = public_bytes.try_into().map_err(|v: Vec<u8>| {
        IpcError::HandshakeFailed {
            reason: format!("{agent_name}.pub: expected 32 bytes, got {}", v.len()),
        }
    })?;

    // Tamper detection.
    let checksum_path = dir.join(format!("{agent_name}.checksum"));
    match tokio::fs::read(&checksum_path).await {
        Ok(stored) => {
            let expected = keypair_checksum(&public_key, &private_bytes);
            if stored.len() != 32 || stored[..] != expected[..] {
                return Err(IpcError::KeyTamperDetected {
                    agent: agent_name.into(),
                    path: checksum_path.display().to_string(),
                });
            }
            tracing::debug!(agent = agent_name, "keypair integrity verified");
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::warn!(
                agent = agent_name,
                "no checksum file — keypair integrity unverifiable"
            );
        }
        Err(e) => return Err(IpcError::Io(e)),
    }

    Ok((zeroize::Zeroizing::new(private_bytes), public_key))
}

/// [RC-4] Atomic private key write: tmp file with 0600 from creation, fsync, rename.
#[cfg(unix)]
async fn write_private_key_atomic(final_path: &Path, private_key: &[u8]) -> Result<()> {
    let tmp_path = final_path.with_extension("key.tmp");
    let private_copy = zeroize::Zeroizing::new(private_key.to_vec());
    let tmp = tmp_path.clone();
    let target = final_path.to_owned();

    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600) // [RC-6] 0600 from creation, no permission window
            .open(&tmp)?;
        f.write_all(&private_copy)?;
        f.sync_all()?;
        std::fs::rename(&tmp, &target)?;
        Ok(())
    })
    .await
    .map_err(|e| IpcError::Io(std::io::Error::other(e)))?
    .map_err(IpcError::Io)
}

#[cfg(not(unix))]
async fn write_private_key_atomic(final_path: &Path, private_key: &[u8]) -> Result<()> {
    // Non-Unix: best-effort write without mode enforcement.
    tokio::fs::write(final_path, private_key)
        .await
        .map_err(IpcError::Io)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_agent_name_accepts_valid() {
        assert!(validate_agent_name("rekindle-tui").is_ok());
        assert!(validate_agent_name("ai_agent_01").is_ok());
        assert!(validate_agent_name("Bot42").is_ok());
    }

    #[test]
    fn validate_agent_name_rejects_invalid() {
        assert!(validate_agent_name("").is_err());
        assert!(validate_agent_name("../escape").is_err());
        assert!(validate_agent_name("has/slash").is_err());
        assert!(validate_agent_name("has space").is_err());
        assert!(validate_agent_name("has.dot").is_err());
    }

    #[test]
    fn keypair_checksum_is_deterministic() {
        let kp = generate_keypair().unwrap();
        let pub_arr: [u8; 32] = kp.public().try_into().unwrap();
        let c1 = keypair_checksum(&pub_arr, kp.private());
        let c2 = keypair_checksum(&pub_arr, kp.private());
        assert_eq!(c1, c2);
    }

    #[test]
    fn keypair_checksum_detects_tampering() {
        let kp = generate_keypair().unwrap();
        let pub_arr: [u8; 32] = kp.public().try_into().unwrap();
        let original = keypair_checksum(&pub_arr, kp.private());
        let mut tampered = kp.private().to_vec();
        tampered[0] ^= 0xFF;
        let tampered_checksum = keypair_checksum(&pub_arr, &tampered);
        assert_ne!(original, tampered_checksum);
    }

    #[test]
    fn generate_keypair_produces_32_byte_keys() {
        let kp = generate_keypair().unwrap();
        assert_eq!(kp.private().len(), 32);
        assert_eq!(kp.public().len(), 32);
    }

    #[test]
    fn zeroizing_keypair_into_inner_preserves_material() {
        let kp = generate_keypair().unwrap();
        let private_copy = kp.private().to_vec();
        let extracted = kp.into_inner();
        assert_eq!(extracted.private, private_copy);
    }
}

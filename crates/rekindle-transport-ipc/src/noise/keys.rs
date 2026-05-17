//! Key management: generation, persistence, tamper detection.
//!
//! Key storage layout under a provided directory:
//! - `bus.pub` (0644), `bus.key` (0600), `bus.checksum`
//! - `keys/<agent>.pub` (0644), `keys/<agent>.key` (0600), `keys/<agent>.checksum`
//!
//! All private key writes use atomic tmp+fsync+rename pattern.
//! Private keys wrapped in `ZeroizingKeypair` for drop cleanup.

use std::path::{Path, PathBuf};

use crate::error::{IpcError, IpcResult};

/// Noise IK protocol parameters.
///
/// AES-256-GCM with SHA-256: uses AES-NI + CLMUL hardware acceleration
/// via aws-lc-rs through our custom snow resolver.
pub const NOISE_PARAMS: &str = "Noise_IK_25519_AESGCM_SHA256";

// ---- Agent name validation ----

/// Validate agent name for filesystem safety. Must match `[a-zA-Z0-9_-]+`.
pub fn validate_agent_name(name: &str) -> IpcResult<()> {
    if name.is_empty() {
        return Err(IpcError::InvalidAgentName {
            name: "(empty)".into(),
        });
    }
    for ch in name.chars() {
        if !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-' {
            return Err(IpcError::InvalidAgentName { name: name.into() });
        }
    }
    Ok(())
}

// ---- BLAKE3 tamper detection ----

/// Integrity checksum: BLAKE3 keyed hash (pubkey as key, privkey as data).
fn keypair_checksum(public_key: &[u8; 32], private_key: &[u8]) -> [u8; 32] {
    *blake3::keyed_hash(public_key, private_key).as_bytes()
}

// ---- ZeroizingKeypair ----

/// Zeroize-on-drop wrapper for `snow::Keypair`.
///
/// snow::Keypair has no Drop impl — private key persists in freed memory.
/// This wrapper guarantees zeroization on drop, including during panics.
pub struct ZeroizingKeypair {
    inner: snow::Keypair,
}

impl ZeroizingKeypair {
    #[must_use]
    pub fn new(keypair: snow::Keypair) -> Self {
        Self { inner: keypair }
    }

    #[must_use]
    pub fn public(&self) -> &[u8] {
        &self.inner.public
    }

    #[must_use]
    pub fn private(&self) -> &[u8] {
        &self.inner.private
    }

    #[must_use]
    pub fn as_inner(&self) -> &snow::Keypair {
        &self.inner
    }

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

// ---- Key generation ----

/// Generate a new X25519 static keypair for Noise IK.
pub fn generate_keypair() -> IpcResult<ZeroizingKeypair> {
    let builder = super::resolver::noise_builder(NOISE_PARAMS);
    let keypair = builder.generate_keypair().map_err(|e| IpcError::HandshakeFailed {
        reason: format!("keypair generation: {e}"),
    })?;
    Ok(ZeroizingKeypair::new(keypair))
}

// ---- Key persistence ----

fn keys_dir(base: &Path) -> PathBuf {
    base.join("keys")
}

/// Create the per-agent keys directory if it does not exist.
pub async fn create_keys_dir(base: &Path) -> IpcResult<()> {
    let dir = keys_dir(base);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| IpcError::DirectoryCreate {
            path: dir.display().to_string(),
            source: e,
        })?;
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

/// Write the bus server's static keypair.
pub async fn write_bus_keypair(keypair: &snow::Keypair, dir: &Path) -> IpcResult<()> {
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| IpcError::DirectoryCreate {
            path: dir.display().to_string(),
            source: e,
        })?;

    let pub_path = dir.join("bus.pub");
    let key_path = dir.join("bus.key");

    tokio::fs::write(&pub_path, &keypair.public).await.map_err(IpcError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(&pub_path, std::fs::Permissions::from_mode(0o644)).await;
    }

    write_private_key_atomic(&key_path, &keypair.private).await?;

    let pub_array: [u8; 32] = keypair.public.clone().try_into().map_err(|_| {
        IpcError::HandshakeFailed {
            reason: "bus public key not 32 bytes".into(),
        }
    })?;
    let checksum = keypair_checksum(&pub_array, &keypair.private);
    tokio::fs::write(dir.join("bus.checksum"), checksum)
        .await
        .map_err(IpcError::Io)?;

    Ok(())
}

/// Write a per-agent keypair to disk.
pub async fn write_agent_keypair(
    agent_name: &str,
    keypair: &snow::Keypair,
    base: &Path,
) -> IpcResult<()> {
    validate_agent_name(agent_name)?;
    let dir = keys_dir(base);

    let pub_path = dir.join(format!("{agent_name}.pub"));
    let key_path = dir.join(format!("{agent_name}.key"));

    tokio::fs::write(&pub_path, &keypair.public).await.map_err(IpcError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(&pub_path, std::fs::Permissions::from_mode(0o644)).await;
    }

    write_private_key_atomic(&key_path, &keypair.private).await?;

    let pub_array: [u8; 32] = keypair.public.clone().try_into().map_err(|_| {
        IpcError::HandshakeFailed {
            reason: format!("{agent_name} public key not 32 bytes"),
        }
    })?;
    let checksum = keypair_checksum(&pub_array, &keypair.private);
    tokio::fs::write(dir.join(format!("{agent_name}.checksum")), checksum)
        .await
        .map_err(IpcError::Io)?;

    Ok(())
}

/// Read the bus server's public key.
pub async fn read_bus_public_key(dir: &Path) -> IpcResult<[u8; 32]> {
    let bytes = tokio::fs::read(dir.join("bus.pub")).await.map_err(IpcError::Io)?;
    bytes.try_into().map_err(|v: Vec<u8>| IpcError::HandshakeFailed {
        reason: format!("bus.pub: expected 32 bytes, got {}", v.len()),
    })
}

/// Read an agent's keypair with tamper detection.
pub async fn read_agent_keypair(
    agent_name: &str,
    base: &Path,
) -> IpcResult<(zeroize::Zeroizing<Vec<u8>>, [u8; 32])> {
    validate_agent_name(agent_name)?;
    let dir = keys_dir(base);

    let private_bytes = tokio::fs::read(dir.join(format!("{agent_name}.key")))
        .await
        .map_err(IpcError::Io)?;
    let public_bytes = tokio::fs::read(dir.join(format!("{agent_name}.pub")))
        .await
        .map_err(IpcError::Io)?;

    let public_key: [u8; 32] = public_bytes.try_into().map_err(|v: Vec<u8>| {
        IpcError::HandshakeFailed {
            reason: format!("{agent_name}.pub: expected 32 bytes, got {}", v.len()),
        }
    })?;

    // Tamper detection via BLAKE3 checksum.
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
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::warn!(agent = agent_name, "no checksum file — integrity unverifiable");
        }
        Err(e) => return Err(IpcError::Io(e)),
    }

    Ok((zeroize::Zeroizing::new(private_bytes), public_key))
}

/// Atomic private key write: tmp + fsync + rename with 0600 from creation.
#[cfg(unix)]
async fn write_private_key_atomic(final_path: &Path, private_key: &[u8]) -> IpcResult<()> {
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
            .mode(0o600)
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
async fn write_private_key_atomic(final_path: &Path, private_key: &[u8]) -> IpcResult<()> {
    tokio::fs::write(final_path, private_key).await.map_err(IpcError::Io)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_agent_name_accepts_valid() {
        assert!(validate_agent_name("rekindle-tui").is_ok());
        assert!(validate_agent_name("ai_agent_01").is_ok());
    }

    #[test]
    fn validate_agent_name_rejects_invalid() {
        assert!(validate_agent_name("").is_err());
        assert!(validate_agent_name("../escape").is_err());
        assert!(validate_agent_name("has space").is_err());
    }

    #[test]
    fn generate_keypair_produces_32_byte_keys() {
        let kp = generate_keypair().unwrap();
        assert_eq!(kp.private().len(), 32);
        assert_eq!(kp.public().len(), 32);
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
        assert_ne!(original, keypair_checksum(&pub_arr, &tampered));
    }
}

//! Key management and persistence for Noise IK keypairs.
//!
//! Handles generation, filesystem I/O, permissions, and tamper-detection
//! checksums. Adapted from open-sesame `core-ipc/src/noise_keys.rs`.
//!
//! Key storage layout under `$XDG_RUNTIME_DIR/rekindle/`:
//! - `bus.pub` (0644) — bus server public key, read by connecting clients.
//!   The bus private key never reaches the disk: the daemon generates it at
//!   each start and holds it in memory (plan D1).
//!
//! Per-agent keys live under the data root's config directory
//! (`DataRoot.config/agent-keys/`, 0700, created by the client): a client's
//! static key must survive a reboot, because the daemon remembers it (the
//! identity's vault holds the public half and its clearance, plan D1).
//! - `agent-keys/<agent>.pub` (0644) — per-agent public key
//! - `agent-keys/<agent>.key` (0600) — per-agent private key (atomic write)
//! - `agent-keys/<agent>.checksum` — per-agent integrity checksum
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
            return Err(IpcError::InvalidAgentName { name: name.into() });
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
pub const NOISE_PARAMS: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";

/// Generate a new X25519 static keypair for Noise IK.
pub fn generate_keypair() -> Result<ZeroizingKeypair> {
    let builder =
        snow::Builder::new(
            NOISE_PARAMS
                .parse()
                .map_err(|e| IpcError::HandshakeFailed {
                    reason: format!("invalid Noise params: {e}"),
                })?,
        );
    let keypair = builder
        .generate_keypair()
        .map_err(|e| IpcError::HandshakeFailed {
            reason: format!("keypair generation failed: {e}"),
        })?;
    Ok(ZeroizingKeypair::new(keypair))
}

// ── Key persistence ────────────────────────────────────────────────────

fn keys_dir() -> Result<PathBuf> {
    let root =
        rekindle_utils::paths::DataRoot::resolve().map_err(|e| IpcError::DirectoryCreate {
            path: "the data root".into(),
            source: std::io::Error::other(e.to_string()),
        })?;
    Ok(root.config.join("agent-keys"))
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

/// Publish the bus server's public key as `bus.pub` (0644), written
/// atomically so a connecting client never reads half of it.
pub async fn write_bus_public_key(public: &[u8; 32]) -> Result<()> {
    let dir = runtime_dir()?;
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| IpcError::DirectoryCreate {
            path: dir.display().to_string(),
            source: e,
        })?;
    let pub_path = dir.join("bus.pub");
    let tmp_path = dir.join("bus.pub.tmp");
    tokio::fs::write(&tmp_path, public)
        .await
        .map_err(IpcError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o644))
            .await
            .map_err(IpcError::Io)?;
    }
    tokio::fs::rename(&tmp_path, &pub_path)
        .await
        .map_err(IpcError::Io)?;
    tracing::info!(pub_path = %pub_path.display(), "bus public key published");
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
    let pub_array: [u8; 32] =
        keypair
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
    let bytes = tokio::fs::read(&pub_path).await.map_err(IpcError::Io)?;
    bytes
        .try_into()
        .map_err(|v: Vec<u8>| IpcError::HandshakeFailed {
            reason: format!("bus.pub: expected 32 bytes, got {}", v.len()),
        })
}

/// The agent's persistent keypair, created on first use. Created means
/// none of its files exists; a partial, malformed or tampered keypair is an
/// error, never silently replaced (a replaced key loses its clearance).
///
/// # Errors
/// An incomplete or damaged keypair, or the files cannot be written.
pub async fn load_or_create_agent_keypair(agent_name: &str) -> Result<ZeroizingKeypair> {
    validate_agent_name(agent_name)?;
    let dir = keys_dir()?;
    let none_exists = ["key", "pub", "checksum"]
        .iter()
        .all(|ext| !dir.join(format!("{agent_name}.{ext}")).exists());
    if none_exists {
        create_keys_dir().await?;
        let keypair = generate_keypair()?;
        write_agent_keypair(agent_name, keypair.as_inner()).await?;
        return Ok(keypair);
    }
    let (private, public) = read_checked_keypair(&dir, agent_name).await?;
    Ok(ZeroizingKeypair::new(snow::Keypair {
        private: private.to_vec(),
        public: public.to_vec(),
    }))
}

/// Read an agent's keypair from disk with tamper detection.
pub async fn read_agent_keypair(
    agent_name: &str,
) -> Result<(zeroize::Zeroizing<Vec<u8>>, [u8; 32])> {
    validate_agent_name(agent_name)?;
    read_checked_keypair(&keys_dir()?, agent_name).await
}

/// Read `<stem>.key`, `<stem>.pub` and `<stem>.checksum` from `dir` and
/// verify the checksum in constant time. Every file is required: a keypair
/// without its checksum is one whose integrity nothing vouches for.
async fn read_checked_keypair(
    dir: &Path,
    stem: &str,
) -> Result<(zeroize::Zeroizing<Vec<u8>>, [u8; 32])> {
    let read = |name: String| async move {
        let path = dir.join(name);
        tokio::fs::read(&path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                IpcError::KeyIncomplete {
                    agent: stem.into(),
                    path: path.display().to_string(),
                }
            } else {
                IpcError::Io(e)
            }
        })
    };
    let malformed = |name: String, got: usize| IpcError::KeyMalformed {
        agent: stem.into(),
        path: dir.join(name).display().to_string(),
        got,
    };
    let private_bytes = zeroize::Zeroizing::new(read(format!("{stem}.key")).await?);
    if private_bytes.len() != 32 {
        return Err(malformed(format!("{stem}.key"), private_bytes.len()));
    }
    let public_key: [u8; 32] = read(format!("{stem}.pub"))
        .await?
        .try_into()
        .map_err(|v: Vec<u8>| malformed(format!("{stem}.pub"), v.len()))?;
    let stored = read(format!("{stem}.checksum")).await?;
    let expected = keypair_checksum(&public_key, &private_bytes);
    if !bool::from(subtle::ConstantTimeEq::ct_eq(&stored[..], &expected[..])) {
        return Err(IpcError::KeyTamperDetected {
            agent: stem.into(),
            path: dir.join(format!("{stem}.checksum")).display().to_string(),
        });
    }
    tracing::debug!(keypair = stem, "keypair integrity verified");
    Ok((private_bytes, public_key))
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
#[path = "noise_keys_tests.rs"]
mod tests;

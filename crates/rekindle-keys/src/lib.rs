//! Application-level key lifecycle: generate, persist, read, tamper-detect.
//!
//! The daemon's Noise IK static keypair is managed here. transport-ipc
//! receives the keypair as input and uses it for handshakes — it does
//! not manage key files on disk.
//!
//! This crate is shared between rekindle-node (daemon) and rekindle-client
//! (CLI/TUI/Tauri). It contains ONLY key management utilities — no
//! transport, no business logic, no daemon state.

use std::env;
use std::fs::Permissions;
use std::mem;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use tokio::fs;
use tracing::debug;
use zeroize::{Zeroize, Zeroizing};

/// Noise protocol parameters. Must match transport-ipc's expectations.
pub const NOISE_PARAMS: &str = "Noise_IK_25519_AESGCM_SHA256";

/// A keypair wrapper that zeroizes the private key on drop.
pub struct ZeroizingKeypair {
    inner: snow::Keypair,
}

impl ZeroizingKeypair {
    pub fn new(kp: snow::Keypair) -> Self {
        Self { inner: kp }
    }

    pub fn as_inner(&self) -> &snow::Keypair {
        &self.inner
    }

    pub fn into_inner(mut self) -> snow::Keypair {
        let private = mem::take(&mut self.inner.private);
        let public = mem::take(&mut self.inner.public);
        mem::forget(self);
        snow::Keypair { private, public }
    }

    pub fn public(&self) -> &[u8] {
        &self.inner.public
    }
}

impl Drop for ZeroizingKeypair {
    fn drop(&mut self) {
        self.inner.private.zeroize();
    }
}

/// Generate a fresh Noise IK keypair (X25519).
pub fn generate_keypair() -> Result<ZeroizingKeypair, String> {
    let params: snow::params::NoiseParams = NOISE_PARAMS
        .parse()
        .map_err(|e| format!("noise params: {e}"))?;
    let builder = snow::Builder::new(params);
    let kp = builder
        .generate_keypair()
        .map_err(|e| format!("keypair gen: {e}"))?;
    debug!("generated fresh Noise IK keypair");
    Ok(ZeroizingKeypair::new(kp))
}

/// Validate an agent name: alphanumeric, hyphens, underscores, 1-64 chars.
pub fn validate_agent_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 64 {
        return Err("agent name must be 1-64 characters".into());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("agent name must be [a-zA-Z0-9_-]+".into());
    }
    Ok(())
}

/// Resolve the daemon's Unix socket path.
pub fn socket_path() -> Result<PathBuf, String> {
    let dir = runtime_dir()?;
    Ok(dir.join("bus.sock"))
}

/// Resolve the runtime directory for ephemeral daemon state.
pub fn runtime_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "linux")]
    {
        if let Ok(xdg) = env::var("XDG_RUNTIME_DIR") {
            return Ok(PathBuf::from(xdg).join("rekindle"));
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = env::var("HOME") {
            return Ok(PathBuf::from(home).join("Library/Application Support/rekindle"));
        }
    }

    let uid = rustix::process::getuid().as_raw();
    Ok(PathBuf::from(format!("/tmp/rekindle-{uid}")))
}

/// Read the daemon's public key from the runtime directory.
pub async fn read_bus_public_key() -> Result<[u8; 32], String> {
    let dir = runtime_dir()?;
    let pub_path = dir.join("bus.pub");
    let bytes = fs::read(&pub_path)
        .await
        .map_err(|e| format!("read {}: {e}", pub_path.display()))?;
    if bytes.len() != 32 {
        return Err(format!("bus.pub is {} bytes, expected 32", bytes.len()));
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    debug!(path = %pub_path.display(), "loaded bus public key");
    Ok(key)
}

/// Write the daemon's keypair to the runtime directory.
pub async fn write_bus_keypair(kp: &snow::Keypair) -> Result<(), String> {
    let dir = runtime_dir()?;
    fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("create runtime dir: {e}"))?;

    atomic_write(&dir.join("bus.pub"), &kp.public, 0o644).await?;
    atomic_write(&dir.join("bus.key"), &kp.private, 0o600).await?;

    let mut hasher = blake3::Hasher::new();
    hasher.update(&kp.public);
    hasher.update(&kp.private);
    let checksum = hasher.finalize();
    atomic_write(&dir.join("bus.checksum"), checksum.as_bytes(), 0o600).await?;

    debug!(path = %dir.display(), "wrote bus keypair with BLAKE3 checksum");
    Ok(())
}

/// Load a bus keypair from disk with BLAKE3 integrity check.
pub async fn load_bus_keypair(
    pub_path: &Path,
    key_path: &Path,
) -> Result<ZeroizingKeypair, String> {
    let public = fs::read(pub_path)
        .await
        .map_err(|e| format!("read pub: {e}"))?;
    let mut private = fs::read(key_path)
        .await
        .map_err(|e| format!("read key: {e}"))?;

    if public.len() != 32 || private.len() != 32 {
        private.zeroize();
        return Err("keypair files wrong size".into());
    }

    let checksum_path = pub_path.with_file_name("bus.checksum");
    if checksum_path.exists() {
        let stored = fs::read(&checksum_path)
            .await
            .map_err(|e| format!("read checksum: {e}"))?;

        let mut hasher = blake3::Hasher::new();
        hasher.update(&public);
        hasher.update(&private);
        let computed = hasher.finalize();

        if stored.len() != 32 || stored[..] != computed.as_bytes()[..] {
            private.zeroize();
            return Err("TAMPER DETECTED: bus keypair checksum mismatch".into());
        }
        debug!(path = %pub_path.display(), "BLAKE3 integrity check passed");
    }

    Ok(ZeroizingKeypair::new(snow::Keypair {
        public,
        private,
    }))
}

/// Ensure the keys directory exists with mode 0o700.
pub async fn create_keys_dir() -> Result<(), String> {
    let dir = keys_dir()?;
    fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("create keys dir: {e}"))?;

    #[cfg(unix)]
    {
        fs::set_permissions(&dir, Permissions::from_mode(0o700))
            .await
            .map_err(|e| format!("chmod keys dir: {e}"))?;
    }

    debug!(path = %dir.display(), "keys directory ready");
    Ok(())
}

/// Read an agent's keypair with BLAKE3 tamper detection.
pub async fn read_agent_keypair(
    name: &str,
) -> Result<(Zeroizing<Vec<u8>>, [u8; 32]), String> {
    validate_agent_name(name)?;
    let dir = keys_dir()?;

    let pub_bytes = fs::read(dir.join(format!("{name}.pub")))
        .await
        .map_err(|e| format!("read agent pub: {e}"))?;
    let key_bytes = fs::read(dir.join(format!("{name}.key")))
        .await
        .map_err(|e| format!("read agent key: {e}"))?;

    let checksum_path = dir.join(format!("{name}.blake3"));
    if checksum_path.exists() {
        let stored = fs::read(&checksum_path)
            .await
            .map_err(|e| format!("read checksum: {e}"))?;

        let mut hasher = blake3::Hasher::new();
        hasher.update(&pub_bytes);
        hasher.update(&key_bytes);
        let computed = hasher.finalize();

        if stored.len() != 32 || stored[..] != computed.as_bytes()[..] {
            return Err(format!(
                "agent '{name}' keypair integrity check failed — possible tampering"
            ));
        }
        debug!(agent = name, "agent keypair BLAKE3 integrity check passed");
    }

    if pub_bytes.len() != 32 {
        return Err(format!("agent pub key wrong size: {}", pub_bytes.len()));
    }

    let mut pub_key = [0u8; 32];
    pub_key.copy_from_slice(&pub_bytes);

    Ok((Zeroizing::new(key_bytes), pub_key))
}

/// Write an agent's keypair to the keys directory.
pub async fn write_agent_keypair(
    name: &str,
    kp: &snow::Keypair,
) -> Result<(), String> {
    validate_agent_name(name)?;
    let dir = keys_dir()?;

    let mut hasher = blake3::Hasher::new();
    hasher.update(&kp.public);
    hasher.update(&kp.private);
    let checksum = hasher.finalize();

    atomic_write(&dir.join(format!("{name}.pub")), &kp.public, 0o644).await?;
    atomic_write(&dir.join(format!("{name}.key")), &kp.private, 0o600).await?;
    atomic_write(
        &dir.join(format!("{name}.blake3")),
        checksum.as_bytes(),
        0o600,
    )
    .await?;

    debug!(agent = name, "wrote agent keypair with BLAKE3 checksum");
    Ok(())
}

fn keys_dir() -> Result<PathBuf, String> {
    let state = env::var("XDG_STATE_HOME").unwrap_or_else(|_| {
        let home = env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        format!("{home}/.local/state")
    });
    Ok(PathBuf::from(state).join("rekindle/keys"))
}

/// Atomic write: write to temp file, set permissions, rename to target.
async fn atomic_write(path: &Path, data: &[u8], _mode: u32) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data)
        .await
        .map_err(|e| format!("write {}: {e}", tmp.display()))?;

    #[cfg(unix)]
    {
        fs::set_permissions(&tmp, Permissions::from_mode(_mode))
            .await
            .map_err(|e| format!("chmod {}: {e}", tmp.display()))?;
    }

    fs::rename(&tmp, path)
        .await
        .map_err(|e| format!("rename {} → {}: {e}", tmp.display(), path.display()))?;

    Ok(())
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
    fn generate_keypair_produces_32_byte_keys() {
        let kp = generate_keypair().unwrap();
        assert_eq!(kp.as_inner().private.len(), 32);
        assert_eq!(kp.as_inner().public.len(), 32);
    }
}

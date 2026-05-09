//! SSH agent challenge-response vault unlock.
//!
//! A deterministic challenge is signed by the SSH key. The signature
//! is BLAKE3-derived to produce a wrapping key. The master key is
//! AES-256-GCM wrapped and stored alongside the challenge in a JSON config.

use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

use crate::error::{StorageError, StorageResult};
use crate::unlock::{MasterKey, VaultUnlock};
use crate::vault::entry_crypto;

#[derive(serde::Serialize, serde::Deserialize)]
struct SshConfig {
    fingerprint: String,
    challenge: Vec<u8>,
    wrapped_master: Vec<u8>,
}

pub struct SshUnlock {
    config_path: PathBuf,
}

impl SshUnlock {
    pub fn new(state_dir: &Path) -> Self {
        Self {
            config_path: state_dir.join("ssh_unlock.json"),
        }
    }

    /// Sign a challenge via the SSH agent.
    ///
    /// Connects to `SSH_AUTH_SOCK`, requests signature of `challenge`
    /// using the key identified by `fingerprint`. Returns raw signature bytes.
    fn sign_challenge(challenge: &[u8], fingerprint: &str) -> StorageResult<Vec<u8>> {
        // Platform-specific SSH agent protocol implementation.
        // The node crate wraps SshUnlock::unlock() in spawn_blocking
        // because this is a synchronous blocking call to the agent socket.
        let _ = (challenge, fingerprint);
        Err(StorageError::SshAgentFailed(
            "SSH agent signing not yet implemented — requires platform ssh-agent protocol".into(),
        ))
    }
}

impl VaultUnlock for SshUnlock {
    fn unlock(&self) -> StorageResult<MasterKey> {
        let json = std::fs::read_to_string(&self.config_path)
            .map_err(|e| StorageError::SshAgentFailed(format!("read: {e}")))?;
        let config: SshConfig = serde_json::from_str(&json)
            .map_err(|e| StorageError::SshAgentFailed(format!("parse: {e}")))?;

        let sig = Self::sign_challenge(&config.challenge, &config.fingerprint)?;
        let derived = blake3::derive_key("rekindle v1 ssh-master", &sig);
        let kek = Zeroizing::new(derived);

        let plain = entry_crypto::decrypt(&kek, &config.wrapped_master)
            .map_err(|_| StorageError::MasterKeyUnwrapFailed)?;

        if plain.len() != 32 {
            return Err(StorageError::MasterKeyUnwrapFailed);
        }
        let mut mk = [0u8; 32];
        mk.copy_from_slice(&plain);
        Ok(MasterKey::from_bytes(mk))
    }

    fn enroll(&self, master_key: &MasterKey) -> StorageResult<()> {
        let mut challenge = vec![0u8; 32];
        aws_lc_rs::rand::SystemRandom::new()
            .fill(&mut challenge)
            .map_err(|e| StorageError::RngFailed(format!("{e}")))?;

        // The fingerprint is provided by the caller (CLI prompts user to select key).
        // For now, enroll writes a placeholder — the caller must set the fingerprint
        // before the first unlock attempt.
        let fingerprint = String::new();

        let sig = Self::sign_challenge(&challenge, &fingerprint)?;
        let derived = blake3::derive_key("rekindle v1 ssh-master", &sig);
        let kek = Zeroizing::new(derived);

        let wrapped = entry_crypto::encrypt(&kek, master_key.as_bytes())?;

        let config = SshConfig {
            fingerprint,
            challenge,
            wrapped_master: wrapped,
        };
        let json = serde_json::to_string_pretty(&config)
            .map_err(|e| StorageError::SshAgentFailed(format!("serialize: {e}")))?;
        std::fs::write(&self.config_path, json)
            .map_err(|e| StorageError::SshAgentFailed(format!("write: {e}")))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(
                &self.config_path,
                std::fs::Permissions::from_mode(0o600),
            );
        }
        Ok(())
    }

    fn revoke(&self) -> StorageResult<()> {
        let _ = std::fs::remove_file(&self.config_path);
        Ok(())
    }

    fn is_available(&self) -> bool {
        std::env::var("SSH_AUTH_SOCK").is_ok() && self.config_path.exists()
    }

    fn id(&self) -> &'static str {
        "ssh"
    }

    fn display_name(&self) -> &'static str {
        "SSH Agent"
    }
}

use aws_lc_rs::rand::SecureRandom;

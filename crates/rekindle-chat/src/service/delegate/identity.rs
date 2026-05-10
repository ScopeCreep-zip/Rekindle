//! Identity delegation — init, destroy, show, export, rotate, wipe.

use crate::ChatError;
use super::super::ChatService;

impl ChatService {
    pub async fn init_identity(
        &self, display_name: &str,
    ) -> Result<crate::identity::IdentityCreated, ChatError> {
        self.identity.init_identity(display_name).await
    }

    pub async fn destroy_identity(&self) -> Result<(), ChatError> {
        self.identity.destroy_identity().await
    }

    /// Return the current session identity metadata (public key, display name, DHT keys).
    /// Returns None if identity is not initialized.
    pub fn identity_show(&self) -> Option<rekindle_types::session_types::SessionIdentity> {
        self.session_meta.read().identity.clone()
    }

    /// Export identity metadata as a serialized blob suitable for file export.
    /// The CLI writes this to a file — the daemon only produces the bytes.
    pub fn identity_export(&self) -> Result<Vec<u8>, ChatError> {
        let identity = self.session_meta.read().identity.clone()
            .ok_or(ChatError::NotInitialized)?;
        serde_json::to_vec_pretty(&identity)
            .map_err(|e| ChatError::Serialization(format!("identity export: {e}")))
    }

    /// Rotate the Ed25519 identity keypair. Generates new key, notifies all friends.
    /// This is a destructive operation — the old public key becomes invalid.
    /// All friends must update their contact for this peer.
    pub async fn identity_rotate(&self) -> Result<(), ChatError> {
        // Identity rotation requires:
        // 1. Generate new Ed25519 seed
        // 2. Store in vault (overwrites old)
        // 3. Set new signing key on PlatformIO
        // 4. Derive new X25519, generate new prekey bundle
        // 5. Update profile DHT with new keys
        // 6. Notify all friends via DM (ProfileKeyRotated)
        // 7. Update session_meta.identity.public_key_hex
        //
        // This is a complex multi-step operation. The underlying
        // IdentityService method handles the full ceremony.
        self.identity.rotate_identity().await
    }

    /// Factory reset — delete all identity, session, vault, config.
    /// Requires typed confirmation matching "WIPE" to prevent accidental invocation.
    ///
    /// Deletes: vault.db, session.json, vault.salt, vault.wrapped,
    /// unlock_methods.json, ssh_unlock.json, audit.jsonl. Clears the
    /// signing key from memory. After this call, the daemon must lock
    /// and the user must run `rekindle init` to create a new identity.
    pub async fn identity_wipe(&self, confirmation: &str) -> Result<(), ChatError> {
        if confirmation != "WIPE" {
            return Err(ChatError::Internal(
                "identity wipe requires confirmation text 'WIPE' — \
                 this operation is irreversible and deletes all local data".into()
            ));
        }

        // Destroy identity (close DHT records, clear session_meta, clear signing key)
        self.identity.destroy_identity().await?;

        // Delete persistent state files
        let state_dir = self.session_path.parent()
            .unwrap_or(std::path::Path::new("."));

        let files_to_delete = [
            self.session_path.clone(),
            state_dir.join("vault.db"),
            state_dir.join("vault.db-wal"),
            state_dir.join("vault.db-shm"),
            state_dir.join("vault.salt"),
            state_dir.join("vault.wrapped"),
            state_dir.join("unlock_methods.json"),
            state_dir.join("ssh_unlock.json"),
            state_dir.join("audit.jsonl"),
        ];

        for path in &files_to_delete {
            if path.exists() {
                if let Err(e) = std::fs::remove_file(path) {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "wipe: failed to delete file — manual cleanup required"
                    );
                } else {
                    tracing::info!(path = %path.display(), "wipe: deleted");
                }
            }
        }

        tracing::info!("identity wipe complete — all local state deleted");
        Ok(())
    }
}

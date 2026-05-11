//! Identity delegation — init, destroy, show, export, rotate, wipe, import.

use base64::Engine;

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

    /// Export identity with passphrase-based encryption (Argon2id + AES-256-GCM).
    ///
    /// Serializes the identity bundle, encrypts it with a key derived from the
    /// passphrase via Argon2id, and returns the ciphertext as a base64 string.
    /// Wire format: [16-byte salt || 12-byte nonce || ciphertext || 16-byte tag]
    ///
    /// The CLI writes the raw bytes to disk with 0o600 permissions.
    pub fn identity_export_encrypted(
        &self, passphrase: &str,
    ) -> Result<String, ChatError> {
        let plaintext = self.identity_export()?;
        let wire = rekindle_storage::vault::entry_crypto::encrypt_with_passphrase(
            passphrase.as_bytes(), &plaintext,
        ).map_err(|e| ChatError::Internal(format!("encrypted export failed: {e}")))?;
        Ok(base64::engine::general_purpose::STANDARD.encode(&wire))
    }

    /// Import identity from a passphrase-encrypted bundle.
    ///
    /// Base64-decodes the wire data, derives the key via Argon2id, decrypts
    /// with AES-256-GCM, deserializes the identity, and installs it into
    /// the session.
    pub fn identity_import_encrypted(
        &self, passphrase: &str, data_b64: &str,
    ) -> Result<(), ChatError> {
        let wire = base64::engine::general_purpose::STANDARD.decode(data_b64)
            .map_err(|e| ChatError::Internal(format!("invalid base64: {e}")))?;
        let plaintext = rekindle_storage::vault::entry_crypto::decrypt_with_passphrase(
            passphrase.as_bytes(), &wire,
        ).map_err(|e| ChatError::Internal(format!("decryption failed (wrong passphrase?): {e}")))?;
        self.identity_import_inner(&plaintext)
    }

    /// Import identity from a plaintext JSON bundle.
    pub fn identity_import(
        &self, data: &str,
    ) -> Result<(), ChatError> {
        self.identity_import_inner(data.as_bytes())
    }

    /// Common import path — deserialize, validate, and install identity from bytes.
    fn identity_import_inner(&self, data: &[u8]) -> Result<(), ChatError> {
        let identity: rekindle_types::session_types::SessionIdentity =
            serde_json::from_slice(data)
                .map_err(|e| ChatError::Internal(format!("invalid identity bundle: {e}")))?;

        // Semantic validation — catch malformed bundles before they silently
        // break the session on the next DM/community operation.
        Self::validate_identity(&identity)?;

        let mut meta = self.session_meta.write();
        meta.identity = Some(identity);
        tracing::info!("identity imported successfully");
        Ok(())
    }

    /// Validate structural invariants of an imported identity bundle.
    fn validate_identity(id: &rekindle_types::session_types::SessionIdentity) -> Result<(), ChatError> {
        fn check_hex(field: &str, value: &str, expected_bytes: usize) -> Result<(), ChatError> {
            if value.is_empty() {
                return Err(ChatError::Internal(format!("identity import: {field} is empty")));
            }
            if value.len() != expected_bytes * 2 {
                return Err(ChatError::Internal(format!(
                    "identity import: {field} has wrong length ({} chars, expected {} for {expected_bytes} bytes)",
                    value.len(), expected_bytes * 2,
                )));
            }
            hex::decode(value).map_err(|e| ChatError::Internal(format!(
                "identity import: {field} is not valid hex: {e}"
            )))?;
            Ok(())
        }

        fn check_non_empty(field: &str, value: &str) -> Result<(), ChatError> {
            if value.trim().is_empty() {
                return Err(ChatError::Internal(format!("identity import: {field} is empty")));
            }
            Ok(())
        }

        // Ed25519 public key = 32 bytes = 64 hex chars
        check_hex("public_key_hex", &id.public_key_hex, 32)?;
        check_non_empty("display_name", &id.display_name)?;
        // DHT keys are Veilid TypedKey strings — not raw hex, but must be non-empty
        check_non_empty("profile_dht_key", &id.profile_dht_key)?;
        check_non_empty("mailbox_dht_key", &id.mailbox_dht_key)?;
        check_non_empty("friend_list_dht_key", &id.friend_list_dht_key)?;
        check_non_empty("friend_inbox_key", &id.friend_inbox_key)?;
        // Friend inbox keypair = X25519 secret + public = 64 bytes = 128 hex chars
        check_hex("friend_inbox_keypair_hex", &id.friend_inbox_keypair_hex, 64)?;

        Ok(())
    }
}

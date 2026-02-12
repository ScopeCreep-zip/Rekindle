use crate::CryptoError;

/// Trait for key storage — abstracts over the actual backend.
///
/// The Tauri app implements this using `tauri-plugin-stronghold`.
/// The crypto crate has zero Tauri dependency; it only defines the trait.
pub trait Keychain: Send + Sync {
    /// Store a key under a vault/key pair.
    fn store_key(&self, vault: &str, key: &str, data: &[u8]) -> Result<(), CryptoError>;

    /// Retrieve a key from a vault/key pair.
    fn load_key(&self, vault: &str, key: &str) -> Result<Option<Vec<u8>>, CryptoError>;

    /// Delete a key from a vault/key pair.
    fn delete_key(&self, vault: &str, key: &str) -> Result<(), CryptoError>;

    /// Check if a key exists.
    fn key_exists(&self, vault: &str, key: &str) -> Result<bool, CryptoError>;
}

// Vault and key constants used throughout the application.

/// Vault for identity keys.
pub const VAULT_IDENTITY: &str = "identity";
/// Ed25519 signing private key.
pub const KEY_ED25519_PRIVATE: &str = "ed25519_private";
/// X25519 Diffie-Hellman private key.
pub const KEY_X25519_PRIVATE: &str = "x25519_private";

/// Vault for Signal Protocol keys.
pub const VAULT_SIGNAL: &str = "signal";
/// Signal identity keypair.
pub const KEY_SIGNAL_IDENTITY: &str = "identity_keypair";
/// Current signed prekey.
pub const KEY_SIGNED_PREKEY: &str = "signed_prekey";
/// Batch of one-time prekeys.
pub const KEY_PREKEY_BATCH: &str = "prekey_batch";

/// Vault for Veilid keys.
pub const VAULT_VEILID: &str = "veilid";
/// Veilid protected store encryption key.
pub const KEY_PROTECTED_STORE: &str = "protected_store_key";

/// Vault for community MEKs.
pub const VAULT_COMMUNITIES: &str = "communities";

/// Generate the MEK key name for a community.
pub fn mek_key_name(community_id: &str) -> String {
    format!("mek_{community_id}")
}

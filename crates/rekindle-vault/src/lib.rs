#![forbid(unsafe_code)]
//! Double-encrypted SQLCipher vault for Rekindle.
//!
//! Replaces Stronghold (`iota_stronghold`) as the on-disk store for
//! identity secrets, MEKs, slot keypairs, and Signal session state.
//!
//! ## Encryption layers
//!
//! `Argon2id(passphrase, salt)` with the parameters in the vault header
//! gives a 64-byte master; BLAKE3 `derive_key` under three labels
//! (`domains::VAULT_{SQLCIPHER,ENTRY,APP_DB}_KEY`) gives:
//!
//! 1. **SQLCipher** key: the whole vault file (AES-256-CBC per page).
//! 2. **Per-entry AES-256-GCM** key: every `entries.ciphertext` row, with
//!    AAD `namespace ‖ 0x00 ‖ key` binding it to its address.
//! 3. The **app-db key** ([`AppDbKey`]): the identity's SQLCipher database
//!    (plan D1), so one passphrase unlocks both.
//!
//! ## Header
//!
//! The Argon2id parameters and 32-byte salt live in `{vault}.hdr`
//! ([`header`]), readable before decryption; neither is secret.
//! [`VaultStore::create`] writes it; [`VaultStore::open`] never creates.

//! ## Layout
//!
//! ```sql
//! CREATE TABLE entries (
//!   namespace TEXT NOT NULL,
//!   key TEXT NOT NULL,
//!   nonce BLOB NOT NULL,
//!   ciphertext BLOB NOT NULL,
//!   PRIMARY KEY (namespace, key)
//! );
//! ```
//!
//! Phase 2 of the decomposed-harvest plan; see
//! `/Users/kali/.claude/plans/memoized-dazzling-torvalds.md` § Phase 2.

pub mod error;
pub mod header;
pub mod key;
pub mod schema;
pub mod store;
pub mod typed;

pub use error::VaultError;
pub use key::VaultKey;
pub use store::{AppDbKey, VaultStore};

/// The unlocked identity's vault, shared by everything that reads or
/// writes secrets for it; `None` while no identity is unlocked (plan D1:
/// one handle type for both hosts).
pub type VaultHandle = std::sync::Arc<parking_lot::Mutex<Option<VaultStore>>>;

/// A handle with no vault open.
#[must_use]
pub fn new_handle() -> VaultHandle {
    std::sync::Arc::new(parking_lot::Mutex::new(None))
}

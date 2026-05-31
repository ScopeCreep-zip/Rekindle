#![forbid(unsafe_code)]
//! Double-encrypted SQLCipher vault for Rekindle.
//!
//! Replaces Stronghold (`iota_stronghold`) as the on-disk store for
//! identity secrets, MEKs, slot keypairs, and Signal session state.
//!
//! ## Encryption layers
//!
//! 1. **SQLCipher** (AES-256-CBC at the page level). Encrypts the entire
//!    database file. Key derived from `Argon2id(passphrase, salt)` →
//!    `BLAKE3-keyed("rekindle v1 vault-sqlcipher", master)`.
//! 2. **Per-entry AES-256-GCM**. Every `entries.ciphertext` row is sealed
//!    independently with a key derived from
//!    `BLAKE3-keyed("rekindle v1 vault-entry-gcm", master)`.
//!
//! ## Salt storage
//!
//! The 32-byte salt cannot live inside the SQLCipher database (it would
//! be needed before decryption to derive the key). It lives in a sidecar
//! file `{vault_path}.salt` — plaintext, since salts only need to be
//! unique per install, not secret.
//!
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
pub mod schema;
pub mod store;

pub use error::VaultError;
pub use store::VaultStore;

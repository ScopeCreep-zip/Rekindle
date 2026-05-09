//! Encrypted persistent storage for the Rekindle messaging daemon.
//!
//! All secret material that survives process death is managed by this crate.
//! No other crate reads or writes secret data to disk. No other crate opens
//! SQLCipher. No other crate touches the OS keyring.
//!
//! **Synchronous.** Every operation in this crate is blocking. The node crate
//! wraps calls in `tokio::task::spawn_blocking` when needed. SQLite is
//! synchronous. Keyring ops are synchronous. File I/O is synchronous.
//!
//! **Double encryption.** The vault uses SQLCipher (page-level AES-256-CBC)
//! plus per-entry AES-256-GCM with an independently derived key. Compromise
//! of one layer does not compromise the other.

pub mod error;
pub mod vault;
pub mod keys;
pub mod sessions;
pub mod messages;
pub mod friends;
pub mod mek;
pub mod unlock;
pub mod session_meta;
pub mod platform;
pub mod audit;

pub use error::{StorageError, StorageResult};
pub use vault::VaultStore;
pub use unlock::{MasterKey, VaultUnlock};

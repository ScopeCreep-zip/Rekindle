//! Signal/PQXDH session establishment and the Double Ratchet, built on
//! `rekindle-secrets`' raw key material.

// No unsafe code anywhere in this crate (verified by grep before adding
// this); locking it at the crate root means a future contributor adding
// one has to deliberately remove this line, not just happen not to
// trip a convention.
#![forbid(unsafe_code)]

pub mod bytes;
pub mod dht_crypto;
pub mod error;
pub mod group;
pub mod identity;
pub mod signal;

pub use dht_crypto::DhtRecordKey;
pub use error::CryptoError;
pub use identity::Identity;
pub use signal::SignalSessionManager;

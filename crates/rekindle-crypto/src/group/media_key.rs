//! Media Encryption Key — re-exported from Tier 2.
//!
//! This module used to hold a second `MediaEncryptionKey` and a second
//! `ChannelAad`, and `rekindle-transport` held a third of the former as
//! `Mek`. Three implementations of one 40-byte wire format
//! (`[generation LE(8) || key(32)]`), of which this crate's was the
//! only one carrying the 65-byte provenance suffix.
//!
//! That asymmetry was not free. The daemon\'s `MekCacheAdapter` had to
//! keep election ranks in a side map, because converting into the base
//! form dropped the provenance that
//! `convergence::incoming_wins_same_generation` needs to resolve a
//! same-generation split-brain. One type carrying its own provenance
//! removes the workaround along with the duplication.
//!
//! Canonical home is `rekindle-secrets` per rule B1: crypto lives in
//! Tier 2 and nowhere else.

pub use rekindle_secrets::keys::{ChannelAad, MediaEncryptionKey};

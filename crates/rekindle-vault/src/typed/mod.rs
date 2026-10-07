//! Typed helpers over [`crate::VaultStore`]: one module per kind of
//! secret, so every host stores and reads them the same way (plan C5.4,
//! `steps-10-19.md` N1).

pub mod audit;
pub mod community_keys;
pub mod mek;
pub mod signal;

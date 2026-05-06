//! Community-specific type definitions and constants.
//!
//! These are the structured types stored in governance manifest subkeys,
//! member registry entries, and channel records. Separated from
//! `payload::dht_types` because they are community-domain-specific
//! rather than generic DHT infrastructure.

pub mod permissions;
pub mod audit_log;
pub mod automod;
pub mod onboarding;

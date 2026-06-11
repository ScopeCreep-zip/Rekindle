//! Layer 3 — Locator. Substrate-addressed pointers for network routing.
//!
//! Locators route; they never identify. No Layer-3 value participates
//! in any KDF input, vault label, hash, signature, or equality
//! comparison that establishes identity.

pub mod kinds;
pub mod record;

pub use kinds::{Substrate, ProfileLocator, MailboxLocator, InboxLocator, GovernanceKey};
pub use record::{LocatorKind, LocatorEntry, LocatorRecord};

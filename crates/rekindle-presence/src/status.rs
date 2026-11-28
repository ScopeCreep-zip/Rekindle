//! User-status vocabulary.
//!
//! This crate used to declare its own `UserStatusKind`, justified as
//! keeping "the crate free of the Tauri dependency" — but the real
//! alternative was never src-tauri's `UserStatus`, it was Tier 1's
//! [`rekindle_types::presence::SessionStatus`], which this crate already
//! depends on. The two were variant-for-variant identical down to the
//! Invisible→"offline" fold, and `community/policy.rs` in this very
//! crate was already using `SessionStatus` directly: one crate, two
//! names for one type.
//!
//! Kept as an alias rather than renamed at 73 callsites — the type is
//! what had to converge, not the spelling.

pub use rekindle_types::presence::{SessionStatus as UserStatusKind, INVISIBLE_WIRE_VALUE};

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;

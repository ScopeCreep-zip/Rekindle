//! Reader-validated bounds (plan E3.0): the value ranges a governance entry
//! must respect whoever wrote it.
//!
//! These are facts about a well-formed entry, not permissions: the merge
//! drops an out-of-bounds entry before ordering (so it can neither apply
//! nor become the genesis), `validate_write` refuses it before the creator
//! bypass, `write_entry` names the refusal, and the daemon's IPC boundary
//! applies the same predicates. Each bound has one source, the constant in
//! `rekindle-types` the predicate cites; length is counted in chars by
//! [`key_format::text`], identically at every caller.

use rekindle_types::channel::MAX_SLOWMODE_SECONDS;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::key_format::{self, KeyFormatError, MAX_DESCRIPTION_LEN};
use rekindle_types::member::MAX_TIMEOUT_SECONDS;

/// A channel's slowmode interval: `0..=MAX_SLOWMODE_SECONDS` (0 turns it off).
pub fn slowmode_ok(seconds: u32) -> bool {
    seconds <= MAX_SLOWMODE_SECONDS
}

/// A member timeout: `1..=MAX_TIMEOUT_SECONDS` (a zero timeout is no timeout).
pub fn timeout_ok(duration_seconds: u64) -> bool {
    (1..=MAX_TIMEOUT_SECONDS).contains(&duration_seconds)
}

/// A community description: [`key_format::text`] within
/// [`MAX_DESCRIPTION_LEN`] chars, no control characters but `\n` and `\t`,
/// no hidden format characters. The `Err` names why, for the IPC boundary.
pub fn description(s: &str) -> Result<&str, KeyFormatError> {
    key_format::text(s, MAX_DESCRIPTION_LEN)
}

/// [`description`] as a predicate.
pub fn description_ok(s: &str) -> bool {
    description(s).is_ok()
}

/// The field of `entry` that is out of bounds, or `None` when every bounded
/// field is within its bound. Only `ChannelUpdated.slowmode_seconds`,
/// `TimeoutEntry.duration_seconds` and `CommunityMeta.description` are
/// bounded; every other variant passes.
pub fn out_of_bounds_field(entry: &GovernanceEntry) -> Option<&'static str> {
    match entry {
        GovernanceEntry::ChannelUpdated {
            slowmode_seconds: Some(seconds),
            ..
        } if !slowmode_ok(*seconds) => Some("slowmode_seconds"),
        GovernanceEntry::TimeoutEntry {
            duration_seconds, ..
        } if !timeout_ok(*duration_seconds) => Some("duration_seconds"),
        GovernanceEntry::CommunityMeta {
            description: Some(text),
            ..
        } if !description_ok(text) => Some("description"),
        _ => None,
    }
}

/// Whether every bounded field of `entry` is within its bound.
pub fn entry_within_bounds(entry: &GovernanceEntry) -> bool {
    out_of_bounds_field(entry).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges() {
        assert!(slowmode_ok(0) && slowmode_ok(MAX_SLOWMODE_SECONDS));
        assert!(!slowmode_ok(MAX_SLOWMODE_SECONDS + 1));
        assert!(!timeout_ok(0) && timeout_ok(1) && timeout_ok(MAX_TIMEOUT_SECONDS));
        assert!(!timeout_ok(MAX_TIMEOUT_SECONDS + 1));
        assert!(description_ok(&"é".repeat(MAX_DESCRIPTION_LEN)));
        assert!(!description_ok(&"é".repeat(MAX_DESCRIPTION_LEN + 1)));
        assert!(description_ok("line\n\ttab"));
        assert!(!description_ok("bell\u{7}"));
        assert!(!description_ok("bidi\u{202e}"));
    }
}

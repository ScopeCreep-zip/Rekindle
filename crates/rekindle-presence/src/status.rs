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
mod tests {
    use super::UserStatusKind;

    /// The privacy rule, pinned here because this crate is where
    /// presence rows are written: Invisible must be indistinguishable
    /// from Offline on the wire.
    #[test]
    fn invisible_is_indistinguishable_from_offline() {
        assert_eq!(
            UserStatusKind::Invisible.as_wire_str(),
            UserStatusKind::Offline.as_wire_str()
        );
        assert_eq!(UserStatusKind::Invisible.as_wire_str(), "offline");
    }

    /// …but it is still *functionally* online locally, which is the
    /// whole point of the state.
    #[test]
    fn invisible_is_not_treated_as_available() {
        assert!(!UserStatusKind::Invisible.is_visible_online());
        assert!(!UserStatusKind::Invisible.is_actively_engaged());
    }

    #[test]
    fn availability_predicates() {
        assert!(UserStatusKind::Online.is_visible_online());
        assert!(UserStatusKind::Away.is_visible_online());
        assert!(UserStatusKind::Busy.is_visible_online());
        assert!(!UserStatusKind::Offline.is_visible_online());

        assert!(UserStatusKind::Online.is_actively_engaged());
        assert!(!UserStatusKind::Away.is_actively_engaged());
        assert!(UserStatusKind::Busy.is_actively_engaged());
    }
}

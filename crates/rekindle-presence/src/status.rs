//! User-status enum + wire-string mapping.
//!
//! Mirrors src-tauri's `crate::state::UserStatus` shape but keeps the
//! crate free of the Tauri dependency. The src-tauri adapter does the
//! one-to-one mapping at the boundary.

use serde::{Deserialize, Serialize};

/// Architecture §13.4 — "invisible" appears offline to others, so the
/// wire payload uses `"offline"` for both Invisible and Offline.
pub const INVISIBLE_WIRE_VALUE: &str = "offline";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserStatusKind {
    #[default]
    Online,
    Away,
    Busy,
    Offline,
    /// Online but appears offline to others — can receive but is hidden.
    Invisible,
}

impl UserStatusKind {
    /// Wire-format string sent in `MemberPresence.status`.
    #[must_use]
    pub fn as_wire_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Away => "away",
            Self::Busy => "busy",
            Self::Offline | Self::Invisible => INVISIBLE_WIRE_VALUE,
        }
    }

    /// `true` when peers should consider this status "available"
    /// (receive routing decisions + mention escalation).
    #[must_use]
    pub fn is_visible_online(self) -> bool {
        matches!(self, Self::Online | Self::Away | Self::Busy)
    }

    /// `true` when the user is actively present at the keyboard
    /// (presence indicators flash, notifications can wake them).
    #[must_use]
    pub fn is_actively_engaged(self) -> bool {
        matches!(self, Self::Online | Self::Busy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_str_round_trip() {
        assert_eq!(UserStatusKind::Online.as_wire_str(), "online");
        assert_eq!(UserStatusKind::Away.as_wire_str(), "away");
        assert_eq!(UserStatusKind::Busy.as_wire_str(), "busy");
        assert_eq!(UserStatusKind::Offline.as_wire_str(), "offline");
        assert_eq!(UserStatusKind::Invisible.as_wire_str(), "offline");
    }

    #[test]
    fn is_visible_online_excludes_offline_and_invisible() {
        assert!(UserStatusKind::Online.is_visible_online());
        assert!(UserStatusKind::Away.is_visible_online());
        assert!(UserStatusKind::Busy.is_visible_online());
        assert!(!UserStatusKind::Offline.is_visible_online());
        assert!(!UserStatusKind::Invisible.is_visible_online());
    }

    #[test]
    fn is_actively_engaged_excludes_away_too() {
        assert!(UserStatusKind::Online.is_actively_engaged());
        assert!(UserStatusKind::Busy.is_actively_engaged());
        assert!(!UserStatusKind::Away.is_actively_engaged());
        assert!(!UserStatusKind::Offline.is_actively_engaged());
        assert!(!UserStatusKind::Invisible.is_actively_engaged());
    }

    #[test]
    fn default_is_online() {
        assert_eq!(UserStatusKind::default(), UserStatusKind::Online);
    }

    #[test]
    fn serde_round_trips_lowercase() {
        let json = serde_json::to_string(&UserStatusKind::Away).unwrap();
        assert_eq!(json, "\"away\"");
        let parsed: UserStatusKind = serde_json::from_str("\"busy\"").unwrap();
        assert_eq!(parsed, UserStatusKind::Busy);
    }
}

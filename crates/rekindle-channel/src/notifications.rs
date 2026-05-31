//! Phase 23.D.14 — pure notification primitives ported from
//! `src-tauri/services/community/notifications/`. Architecture §17 +
//! §32 Phase 7 W25.
//!
//! Token-bucket throttle for per-(community, channel) notification
//! rate-limiting (architecture §17.2 line 2402 — 5 emits per 10s
//! window before summary mode kicks in) + `NotificationLevel`
//! tier-cascade primitive + `verify_notification_message` content
//! hash check + `blake3_hex` helper.

use std::collections::{HashMap, VecDeque};
use std::str::FromStr;

use parking_lot::Mutex;

use rekindle_protocol::dht::community::channel_record::ChannelMessage;

/// Architecture §17.2 line 2402 — "Max 5 notifications per channel per
/// 10-second window. If exceeded, a single summary notification
/// replaces the burst." We keep the last-N timestamps per
/// `(community_id, channel_id)` plus a "burst suppressed since
/// timestamp" marker so the next allowed notification can show
/// "N more messages in #channel" instead of the most recent body.
const NOTIFICATION_BURST_LIMIT: usize = 5;
const NOTIFICATION_BURST_WINDOW_MS: u128 = 10_000;

#[derive(Debug, Default)]
struct NotificationWindow {
    /// Recent emit timestamps in milliseconds. Bounded to
    /// `NOTIFICATION_BURST_LIMIT` entries; older entries are popped on
    /// each insert.
    timestamps: VecDeque<u128>,
    /// Number of notifications skipped since the burst started. Reset
    /// to 0 when the next non-suppressed notification fires.
    suppressed_since_emit: u32,
}

#[derive(Debug, Default)]
pub struct NotificationThrottle {
    windows: Mutex<HashMap<(String, String), NotificationWindow>>,
}

/// Outcome of a throttle decision. The caller dispatches one of these
/// to the user — it's never both at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationDecision {
    /// Emit the message-specific notification as normal.
    Emit,
    /// Replace the burst with a single summary
    /// ("N more messages in #channel"). Bundled count is the total
    /// suppressed since the throttle activated, *including* this one.
    EmitSummary { bundled_count: u32 },
    /// We're inside the burst window and the user already saw the
    /// summary — drop silently.
    Drop,
}

impl NotificationThrottle {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn now_ms() -> u128 {
        u128::from(rekindle_utils::time::timestamp_ms())
    }

    /// Architecture §17.2 line 2402 — record an attempt to notify and
    /// return the dispatch decision. Pure logic; takes `now_ms` so
    /// tests can drive the clock.
    pub fn record_attempt(
        &self,
        community_id: &str,
        channel_id: &str,
        now_ms: u128,
    ) -> NotificationDecision {
        let key = (community_id.to_string(), channel_id.to_string());
        let mut guard = self.windows.lock();
        let window = guard.entry(key).or_default();

        let cutoff = now_ms.saturating_sub(NOTIFICATION_BURST_WINDOW_MS);
        while window.timestamps.front().is_some_and(|ts| *ts < cutoff) {
            window.timestamps.pop_front();
        }

        if window.timestamps.len() < NOTIFICATION_BURST_LIMIT {
            window.timestamps.push_back(now_ms);
            if window.suppressed_since_emit > 0 {
                let bundled = window.suppressed_since_emit + 1;
                window.suppressed_since_emit = 0;
                NotificationDecision::EmitSummary {
                    bundled_count: bundled,
                }
            } else {
                NotificationDecision::Emit
            }
        } else {
            window.suppressed_since_emit = window.suppressed_since_emit.saturating_add(1);
            NotificationDecision::Drop
        }
    }

    pub fn record_attempt_now(&self, community_id: &str, channel_id: &str) -> NotificationDecision {
        self.record_attempt(community_id, channel_id, Self::now_ms())
    }
}

/// Architecture §17.1 — three-tier notification level (channel ->
/// community default -> implicit "all"). The string form matches the
/// `level` column in `notification_preferences` and the
/// `CommunityNotificationDefault.level` governance entry field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationLevel {
    All,
    Mentions,
    Nothing,
}

impl NotificationLevel {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Mentions => "mentions",
            Self::Nothing => "nothing",
        }
    }

    #[must_use]
    pub fn to_db(self) -> i64 {
        match self {
            Self::All => 0,
            Self::Mentions => 1,
            Self::Nothing => 2,
        }
    }
}

impl FromStr for NotificationLevel {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "all" => Ok(Self::All),
            "mentions" => Ok(Self::Mentions),
            "nothing" => Ok(Self::Nothing),
            _ => Err("notification level must be one of: all, mentions, nothing".to_string()),
        }
    }
}

pub fn parse_notification_level(level: &str) -> Result<NotificationLevel, String> {
    level.parse()
}

#[must_use]
pub fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Architecture §28.9 — verify the fetched channel-message ciphertext
/// hashes to the announcement payload's `content_hash`. Caller passes
/// the pending fetch's recorded hash; mismatch means the announcer
/// lied or a peer served corrupt bytes.
pub fn verify_message_content_hash(
    expected_content_hash: &str,
    message: &ChannelMessage,
) -> Result<(), &'static str> {
    if blake3_hex(&message.ciphertext) != expected_content_hash {
        return Err("message notification hash mismatch");
    }
    Ok(())
}

#[cfg(test)]
mod throttle_tests {
    use super::*;

    #[test]
    fn first_five_attempts_emit_normally() {
        let throttle = NotificationThrottle::new();
        for i in 0_u128..5 {
            assert_eq!(
                throttle.record_attempt("c", "ch", 1000 + i * 100),
                NotificationDecision::Emit,
                "burst slot {i} should emit",
            );
        }
    }

    #[test]
    fn sixth_attempt_inside_window_drops() {
        let throttle = NotificationThrottle::new();
        for i in 0_u128..5 {
            throttle.record_attempt("c", "ch", 1000 + i * 100);
        }
        assert_eq!(
            throttle.record_attempt("c", "ch", 1500),
            NotificationDecision::Drop,
        );
    }

    #[test]
    fn next_attempt_after_window_emits_summary() {
        let throttle = NotificationThrottle::new();
        for i in 0_u128..5 {
            throttle.record_attempt("c", "ch", 1000 + i * 100);
        }
        throttle.record_attempt("c", "ch", 1500);
        throttle.record_attempt("c", "ch", 1600);
        assert_eq!(
            throttle.record_attempt("c", "ch", 12_000),
            NotificationDecision::EmitSummary { bundled_count: 3 },
        );
        assert_eq!(
            throttle.record_attempt("c", "ch", 12_100),
            NotificationDecision::Emit,
        );
    }

    #[test]
    fn separate_channels_have_separate_windows() {
        let throttle = NotificationThrottle::new();
        for i in 0_u128..5 {
            throttle.record_attempt("c", "ch_a", 1000 + i * 10);
        }
        assert_eq!(
            throttle.record_attempt("c", "ch_b", 1100),
            NotificationDecision::Emit,
        );
        assert_eq!(
            throttle.record_attempt("c", "ch_a", 1100),
            NotificationDecision::Drop,
        );
    }
}

#[cfg(test)]
mod level_tests {
    use super::*;

    #[test]
    fn from_str_round_trip() {
        for level in [
            NotificationLevel::All,
            NotificationLevel::Mentions,
            NotificationLevel::Nothing,
        ] {
            let parsed: NotificationLevel = level.as_str().parse().expect("known level");
            assert_eq!(parsed, level);
        }
    }

    #[test]
    fn from_str_unknown_returns_err() {
        let parsed: Result<NotificationLevel, _> = "loud".parse();
        assert!(parsed.is_err());
    }

    #[test]
    fn parse_rejects_unknown_with_error() {
        let err = parse_notification_level("loud").unwrap_err();
        assert!(err.contains("all"), "error mentions valid options");
    }

    #[test]
    fn to_db_is_stable() {
        assert_eq!(NotificationLevel::All.to_db(), 0);
        assert_eq!(NotificationLevel::Mentions.to_db(), 1);
        assert_eq!(NotificationLevel::Nothing.to_db(), 2);
    }
}

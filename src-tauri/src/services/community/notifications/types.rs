//! Phase 23.D.14 — `NotificationThrottle` + `NotificationDecision` +
//! `NotificationLevel` + token-bucket math + `verify_notification_message`
//! all ported into `rekindle_channel::notifications`. This file is now
//! a re-export shim plus the src-tauri-only `QuietHoursSettings` + db
//! serialisation helpers + `CleartextMentions` borrowed-slice DTO.

pub use rekindle_channel::{NotificationDecision, NotificationLevel, NotificationThrottle};

#[derive(Debug, Clone)]
pub struct QuietHoursSettings {
    pub enabled: bool,
    pub start_hour: u8,
    pub end_hour: u8,
    /// Architecture §17.2 — IANA timezone identifier
    /// (e.g., `"America/Los_Angeles"`). The quiet-hours resolver in
    /// `is_quiet_hours_active` uses `chrono-tz` to compute the correct
    /// local time for the current instant, automatically honoring DST.
    pub timezone: String,
}

impl Default for QuietHoursSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            start_hour: 22,
            end_hour: 7,
            timezone: "UTC".to_string(),
        }
    }
}

impl QuietHoursSettings {
    pub(super) fn from_db(enabled: i64, start_minute: i64, end_minute: i64, timezone: String) -> Self {
        Self {
            enabled: enabled != 0,
            start_hour: u8::try_from(start_minute.div_euclid(60)).unwrap_or(22),
            end_hour: u8::try_from(end_minute.div_euclid(60)).unwrap_or(7),
            timezone,
        }
    }

    pub(super) fn start_minute(&self) -> i64 {
        i64::from(self.start_hour) * 60
    }

    pub(super) fn end_minute(&self) -> i64 {
        i64::from(self.end_hour) * 60
    }
}

/// Cleartext mention metadata pulled directly from the inbound
/// `ChannelMessage` envelope (architecture §28.5 line 3105-3120). The
/// notification resolver consults this without decrypting the body.
/// `flags` is the wire `ChannelMessage.flags` u32 — the resolver reads
/// `MENTION_EVERYONE` / `MENTION_HERE` bits from it.
pub struct CleartextMentions<'a> {
    pub mentioned_pseudonyms: &'a [String],
    pub mentioned_roles: &'a [String],
    pub flags: u32,
}

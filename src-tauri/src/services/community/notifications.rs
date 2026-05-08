use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;

use tauri::Emitter;

use crate::channels::NotificationEvent;
use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::AppState;
use crate::state_helpers;

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
        let mut guard = self
            .windows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let window = guard.entry(key).or_default();

        let cutoff = now_ms.saturating_sub(NOTIFICATION_BURST_WINDOW_MS);
        while window
            .timestamps
            .front()
            .is_some_and(|ts| *ts < cutoff)
        {
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

    pub fn record_attempt_now(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> NotificationDecision {
        self.record_attempt(community_id, channel_id, Self::now_ms())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationLevel {
    All,
    Mentions,
    Nothing,
}

impl NotificationLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Mentions => "mentions",
            Self::Nothing => "nothing",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "all" => Some(Self::All),
            "mentions" => Some(Self::Mentions),
            "nothing" => Some(Self::Nothing),
            _ => None,
        }
    }

    fn to_db(self) -> i64 {
        match self {
            Self::All => 0,
            Self::Mentions => 1,
            Self::Nothing => 2,
        }
    }
}

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
    fn from_db(enabled: i64, start_minute: i64, end_minute: i64, timezone: String) -> Self {
        Self {
            enabled: enabled != 0,
            start_hour: u8::try_from(start_minute.div_euclid(60)).unwrap_or(22),
            end_hour: u8::try_from(end_minute.div_euclid(60)).unwrap_or(7),
            timezone,
        }
    }

    fn start_minute(&self) -> i64 {
        i64::from(self.start_hour) * 60
    }

    fn end_minute(&self) -> i64 {
        i64::from(self.end_hour) * 60
    }
}

pub fn parse_notification_level(level: &str) -> Result<NotificationLevel, String> {
    NotificationLevel::from_str(level)
        .ok_or_else(|| "notification level must be one of: all, mentions, nothing".to_string())
}

/// Architecture §17.1 three-tier cascade resolver. Most-specific wins:
///
/// 1. **Per-channel override** — local-only, stored on `ChannelInfo.notification_level`.
/// 2. **Community default** — governance-broadcast `CommunityNotificationDefault`.
/// 3. **Implicit "all"** — when neither tier 1 nor tier 2 is set.
///
/// User-level DND is layered on top by `should_emit_message_notification`
/// (the quiet-hours short-circuit) and so isn't part of this tier
/// resolution.
pub fn resolve_notification_level(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
) -> NotificationLevel {
    let communities = state.communities.read();
    let Some(community) = communities.get(community_id) else {
        return NotificationLevel::All;
    };

    // Tier 1: per-channel override.
    let channel_level = community
        .channels
        .iter()
        .find(|channel| channel.id == channel_id)
        .map(|channel| channel.notification_level.as_str())
        .and_then(NotificationLevel::from_str);
    if let Some(level) = channel_level {
        return level;
    }

    // Tier 2: community default.
    if let Some(level) = community
        .governance_state
        .as_ref()
        .and_then(|gov| gov.notification_default.as_ref())
        .and_then(|default| NotificationLevel::from_str(&default.level))
    {
        return level;
    }

    // Tier 3: implicit.
    NotificationLevel::All
}

/// Architecture §17.1 tier 1: write a `CommunityNotificationDefault`
/// governance entry so every member learns the community-wide
/// notification default. Per-channel overrides remain local-only and
/// continue to win in `resolve_notification_level`.
pub async fn set_community_default_notification_level(
    state: &Arc<AppState>,
    community_id: &str,
    level: NotificationLevel,
) -> Result<(), String> {
    let lamport = state_helpers::increment_lamport(state, community_id);
    super::governance::write_entry(
        state,
        community_id,
        rekindle_types::governance::GovernanceEntry::CommunityNotificationDefault {
            level: level.as_str().to_string(),
            lamport,
        },
    )
    .await
}

pub fn get_community_default_notification_level(
    state: &Arc<AppState>,
    community_id: &str,
) -> Option<NotificationLevel> {
    let communities = state.communities.read();
    communities
        .get(community_id)
        .and_then(|c| c.governance_state.as_ref())
        .and_then(|gov| gov.notification_default.as_ref())
        .and_then(|d| NotificationLevel::from_str(&d.level))
}

/// Architecture §32 Phase 7 Week 25 — set the notification sound for
/// `(community_id, channel_id)`. Pass `channel_id = ""` to set the
/// community-wide default. `sound_ref = None` removes the override and
/// re-inherits from the next level up.
pub async fn set_notification_sound(
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    channel_id: &str,
    sound_ref: Option<String>,
) -> Result<(), String> {
    // Mirror the channel-level setting into in-memory `ChannelInfo` so
    // `get_community_details` returns the up-to-date value without an
    // extra DB round-trip. Empty `channel_id` means "community default"
    // and is not stored on any per-channel row.
    if !channel_id.is_empty() {
        let mut communities = state.communities.write();
        if let Some(community) = communities.get_mut(community_id) {
            if let Some(channel) = community
                .channels
                .iter_mut()
                .find(|channel| channel.id == channel_id)
            {
                channel.notification_sound_ref.clone_from(&sound_ref);
            }
        }
    }

    let owner_key = state_helpers::current_owner_key(state)?;
    let community_id_owned = community_id.to_string();
    let channel_id_owned = channel_id.to_string();
    db_call(pool, move |conn| {
        // Upsert: notification_preferences may already have a row from
        // set_channel_notification_level. We update sound_ref without
        // disturbing the level column.
        conn.execute(
            "INSERT INTO notification_preferences \
                  (owner_key, community_id, channel_id, level, sound_ref) \
             VALUES (?1, ?2, ?3, 0, ?4) \
             ON CONFLICT(owner_key, community_id, channel_id) \
             DO UPDATE SET sound_ref = excluded.sound_ref",
            rusqlite::params![owner_key, community_id_owned, channel_id_owned, sound_ref],
        )?;
        Ok(())
    })
    .await
}

/// Three-tier resolver mirroring `resolve_notification_level`:
/// channel override → community default → `None` (caller falls back to
/// the app-global `notification_sound: bool` toggle in `app_settings`).
pub async fn resolve_notification_sound(
    pool: &DbPool,
    owner_key: &str,
    community_id: &str,
    channel_id: &str,
) -> Option<String> {
    let owner = owner_key.to_string();
    let cid = community_id.to_string();
    let chid = channel_id.to_string();
    let row: Option<String> = crate::db_helpers::db_call_or_default(pool, move |conn| {
        // Channel override.
        let channel: Option<String> = conn
            .query_row(
                "SELECT sound_ref FROM notification_preferences \
                  WHERE owner_key = ?1 AND community_id = ?2 AND channel_id = ?3",
                rusqlite::params![owner, cid, chid],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        if channel.is_some() {
            return Ok(channel);
        }
        // Community default (channel_id == '').
        let default: Option<String> = conn
            .query_row(
                "SELECT sound_ref FROM notification_preferences \
                  WHERE owner_key = ?1 AND community_id = ?2 AND channel_id = ''",
                rusqlite::params![owner, cid],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        Ok(default)
    })
    .await;
    row
}

pub async fn set_channel_notification_level(
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    channel_id: &str,
    level: NotificationLevel,
) -> Result<(), String> {
    {
        let mut communities = state.communities.write();
        let community = communities
            .get_mut(community_id)
            .ok_or("community not found")?;
        let channel = community
            .channels
            .iter_mut()
            .find(|channel| channel.id == channel_id)
            .ok_or("channel not found")?;
        channel.notification_level = level.as_str().to_string();
    }

    let owner_key = state_helpers::current_owner_key(state)?;
    let community_id_owned = community_id.to_string();
    let channel_id_owned = channel_id.to_string();
    db_call(pool, move |conn| {
        conn.execute(
            "INSERT INTO notification_preferences (owner_key, community_id, channel_id, level)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(owner_key, community_id, channel_id)
             DO UPDATE SET level = excluded.level",
            rusqlite::params![
                owner_key,
                community_id_owned,
                channel_id_owned,
                level.to_db(),
            ],
        )?;
        Ok(())
    })
    .await
}

pub async fn set_quiet_hours(
    state: &Arc<AppState>,
    pool: &DbPool,
    settings: QuietHoursSettings,
) -> Result<(), String> {
    if settings.start_hour > 23 || settings.end_hour > 23 {
        return Err("quiet hours must use 0-23 hours".to_string());
    }
    // Architecture §17.2 — IANA name is the only authoritative source
    // for the local-time computation. Reject unknown zones at write
    // time so the resolver never sees a value it can't parse.
    if settings.timezone.parse::<chrono_tz::Tz>().is_err() {
        return Err(format!("unknown timezone: {}", settings.timezone));
    }

    let owner_key = state_helpers::current_owner_key(state)?;
    db_call(pool, move |conn| {
        conn.execute(
            "INSERT INTO app_settings
             (owner_key, quiet_hours_enabled, quiet_hours_start_minute, quiet_hours_end_minute, quiet_hours_timezone)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(owner_key)
             DO UPDATE SET
               quiet_hours_enabled = excluded.quiet_hours_enabled,
               quiet_hours_start_minute = excluded.quiet_hours_start_minute,
               quiet_hours_end_minute = excluded.quiet_hours_end_minute,
               quiet_hours_timezone = excluded.quiet_hours_timezone",
            rusqlite::params![
                owner_key,
                i32::from(settings.enabled),
                settings.start_minute(),
                settings.end_minute(),
                settings.timezone,
            ],
        )?;
        Ok(())
    })
    .await
}

pub async fn get_quiet_hours(
    state: &Arc<AppState>,
    pool: &DbPool,
) -> Result<QuietHoursSettings, String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    db_call(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT quiet_hours_enabled, quiet_hours_start_minute, quiet_hours_end_minute, quiet_hours_timezone
             FROM app_settings WHERE owner_key = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![owner_key])?;
        if let Some(row) = rows.next()? {
            Ok(QuietHoursSettings::from_db(
                row.get::<_, i64>(0).unwrap_or(0),
                row.get::<_, i64>(1).unwrap_or(1320),
                row.get::<_, i64>(2).unwrap_or(420),
                row.get::<_, String>(3).unwrap_or_else(|_| "UTC".to_string()),
            ))
        } else {
            Ok(QuietHoursSettings::default())
        }
    })
    .await
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

pub async fn should_emit_message_notification(
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    channel_id: &str,
    sender_pseudonym_hex: &str,
    cleartext: CleartextMentions<'_>,
) -> Result<bool, String> {
    use rekindle_types::channel::flags::{MENTION_EVERYONE, MENTION_HERE, SUPPRESS_NOTIFICATIONS};

    // Architecture §32 Phase 7 Week 25 — Do Not Disturb suppresses
    // every notification regardless of the channel level, mention
    // status, or quiet-hours window. Checked first so the rest of the
    // resolution work is skipped under DND.
    if is_do_not_disturb_active(state, pool).await {
        return Ok(false);
    }
    // SUPPRESS_NOTIFICATIONS is per-message (sender opted-out of
    // pinging anyone) — also takes precedence over mentions.
    if cleartext.flags & SUPPRESS_NOTIFICATIONS != 0 {
        return Ok(false);
    }
    if is_quiet_hours_active(state, pool).await? {
        return Ok(false);
    }

    let level = resolve_notification_level(state, community_id, channel_id);

    // Architecture §28.5 + §9.3 (reader-validates): rebuild a
    // `MentionMatches` from the cleartext envelope fields, then strip
    // privileged classes (@everyone/@here) when the sender lacks
    // `MENTION_EVERYONE`. Body text is preserved as written; only the
    // *escalation* effect is gated.
    let mention_everyone = cleartext.flags & MENTION_EVERYONE != 0;
    let mention_here = cleartext.flags & MENTION_HERE != 0;
    let mut mentions = super::mentions::matches_from_cleartext(
        state,
        community_id,
        cleartext.mentioned_pseudonyms,
        cleartext.mentioned_roles,
        mention_everyone,
        mention_here,
    );
    super::mentions::validate_sender_permissions(
        state,
        community_id,
        sender_pseudonym_hex,
        &mut mentions,
    );
    let mentioned = super::mentions::local_member_is_mentioned(state, community_id, &mentions);

    Ok(match level {
        NotificationLevel::All => true,
        NotificationLevel::Mentions | NotificationLevel::Nothing => mentioned,
    })
}

pub async fn emit_message_notification(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    channel_id: &str,
    sender_pseudonym: &str,
    body: &str,
) {
    let decision = state
        .notification_throttle
        .record_attempt_now(community_id, channel_id);
    if matches!(decision, NotificationDecision::Drop) {
        return;
    }

    let (community_name, channel_name) = {
        let communities = state.communities.read();
        let community = communities.get(community_id);
        let community_name = community.map_or_else(
            || "Community".to_string(),
            |community| community.name.clone(),
        );
        let channel_name = community
            .and_then(|community| {
                community
                    .channels
                    .iter()
                    .find(|channel| channel.id == channel_id)
                    .map(|channel| channel.name.clone())
            })
            .unwrap_or_else(|| "channel".to_string());
        (community_name, channel_name)
    };

    let owner_key = state_helpers::owner_key_or_default(state);
    let sound_ref = if owner_key.is_empty() {
        None
    } else {
        resolve_notification_sound(pool, &owner_key, community_id, channel_id).await
    };

    let (title, payload_body) = match decision {
        NotificationDecision::EmitSummary { bundled_count } => {
            let title = format!("#{channel_name}");
            let body = format!(
                "[{community_name}] {bundled_count} more messages in #{channel_name}"
            );
            (title, body)
        }
        NotificationDecision::Emit => {
            let sender_name = sender_pseudonym.chars().take(8).collect::<String>();
            let title = format!("{sender_name} in #{channel_name}");
            let body = format!("[{community_name}] {body}");
            (title, body)
        }
        NotificationDecision::Drop => unreachable!("Drop returned early above"),
    };

    let _ = app_handle.emit(
        "notification-event",
        &NotificationEvent::MessageReceived {
            title,
            body: payload_body,
            community_id: community_id.to_string(),
            channel_id: channel_id.to_string(),
            sound_ref,
        },
    );
}

/// Read the global Do Not Disturb flag for the current user. Falls
/// back to `false` when unset or on any DB error so a malfunctioning
/// settings table can never silently swallow notifications.
pub async fn is_do_not_disturb_active(state: &Arc<AppState>, pool: &DbPool) -> bool {
    let Ok(owner_key) = state_helpers::current_owner_key(state) else {
        return false;
    };
    crate::db_helpers::db_call_or_default(pool, move |conn| {
        conn.query_row(
            "SELECT do_not_disturb FROM app_settings WHERE owner_key = ?1",
            rusqlite::params![owner_key],
            |r| r.get::<_, i64>(0).map(|v| v != 0),
        )
        .or(Ok(false))
    })
    .await
}

/// Toggle the global Do Not Disturb flag for the current user.
pub async fn set_do_not_disturb(
    state: &Arc<AppState>,
    pool: &DbPool,
    enabled: bool,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    let value = i64::from(enabled);
    db_call(pool, move |conn| {
        conn.execute(
            "INSERT INTO app_settings (owner_key, do_not_disturb) \
             VALUES (?1, ?2) \
             ON CONFLICT(owner_key) DO UPDATE SET do_not_disturb = excluded.do_not_disturb",
            rusqlite::params![owner_key, value],
        )?;
        Ok(())
    })
    .await
}

async fn is_quiet_hours_active(state: &Arc<AppState>, pool: &DbPool) -> Result<bool, String> {
    use chrono::{Timelike, Utc};

    let settings = get_quiet_hours(state, pool).await?;
    if !settings.enabled {
        return Ok(false);
    }

    // Architecture §17.2 — IANA zone drives the local-time computation
    // via `chrono-tz`. `with_timezone` produces the unambiguous wall
    // clock for the current UTC instant in the configured zone (UTC→local
    // conversions are never ambiguous; only local→UTC is, which we
    // never do here).
    let tz: chrono_tz::Tz = settings
        .timezone
        .parse()
        .map_err(|e| format!("invalid timezone {}: {e}", settings.timezone))?;
    let local = Utc::now().with_timezone(&tz);
    let local_minutes = i64::from(local.hour()) * 60 + i64::from(local.minute());
    let start_minute = settings.start_minute();
    let end_minute = settings.end_minute();

    Ok(match start_minute.cmp(&end_minute) {
        std::cmp::Ordering::Equal => true,
        std::cmp::Ordering::Less => {
            local_minutes >= start_minute && local_minutes < end_minute
        }
        std::cmp::Ordering::Greater => {
            local_minutes >= start_minute || local_minutes < end_minute
        }
    })
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
        // Two more drops while inside the window.
        throttle.record_attempt("c", "ch", 1500);
        throttle.record_attempt("c", "ch", 1600);
        // Cross the 10s boundary — the next emit should bundle the
        // suppressed pair plus itself (3 total).
        assert_eq!(
            throttle.record_attempt("c", "ch", 12_000),
            NotificationDecision::EmitSummary { bundled_count: 3 },
        );
        // After the summary the bundle counter is cleared.
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
        // ch_a is full; ch_b should still emit.
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


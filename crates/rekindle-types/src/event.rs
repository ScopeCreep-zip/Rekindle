//! Architecture §21 — scheduled-event metadata.
//!
//! Each `GovernanceEntry::EventCreated` writes one of these documents.
//! Stored verbatim in `GovernanceState.events`; LWW per `event_id`.

use serde::{Deserialize, Serialize};

/// Where a scheduled event takes place. `External` carries an opaque
/// URL/string the UI renders as a link; `InGame` references a game
/// title (the rich-presence id) and an optional server address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum EventLocation {
    /// Event happens in a voice channel of this community.
    VoiceChannel(crate::id::ChannelId),
    /// Event happens in a stage channel of this community.
    StageChannel(crate::id::ChannelId),
    /// External link (Twitch, YouTube, Discord, etc.).
    External(String),
    /// In-game meetup. `game_id` is the rich-presence game identifier;
    /// `server_address` is the IP/host:port string the launcher uses.
    InGame {
        game_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        server_address: Option<String>,
    },
}

/// Lifecycle state of a scheduled event. Transitions are written by the
/// event creator (or anyone with `MANAGE_EVENTS`) as new
/// `EventCreated` entries with bumped `lamport` so LWW resolves them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventStatus {
    Scheduled,
    Active,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecurrenceFrequency {
    Daily,
    Weekly,
    Monthly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DayOfWeek {
    Sunday,
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
}

/// Recurrence per RFC 5545 (iCalendar) `RRULE` semantics, simplified
/// to the subset architecture §21 calls out. `interval` is the gap
/// between recurrences in `frequency` units (e.g. `frequency: Weekly,
/// interval: 2` = every other week). `days_of_week` is meaningful
/// only for `Weekly`. Either `until` or `count` may bound the
/// recurrence (or neither — in which case it repeats forever).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecurrenceRule {
    pub frequency: RecurrenceFrequency,
    pub interval: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub days_of_week: Option<Vec<DayOfWeek>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
}

/// Maximum lengths from spec §21 line 2622-2623.
pub const MAX_EVENT_NAME_CHARS: usize = 100;
pub const MAX_EVENT_DESCRIPTION_CHARS: usize = 1000;

/// One RSVP entry attached to an `EventInfo`. String fields preserve
/// the existing on-the-wire shape used by `EventRsvpChanged` and the
/// `BootstrapResponse` event broadcasts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventRsvp {
    /// Hex-encoded community pseudonym.
    pub pseudonym_key: String,
    /// `going` / `interested` / `declined` (architecture §21 line 2643).
    pub status: String,
}

/// Full event document broadcast over `EventCreated` / `EventUpdated`
/// envelopes. Wire shape matches the pre-migration JSON form so the
/// envelope swap to typed Rust is wire-compatible; the Cap'n Proto
/// migration (`.claude/plans/community-envelope-capnp-migration.md`,
/// Phase 5) replaces these strings with typed-id schemas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventInfo {
    /// `evt_<32 hex>` — 16-byte UUID.
    pub id: String,
    pub title: String,
    pub description: String,
    /// Hex-encoded creator pseudonym.
    pub creator_pseudonym: String,
    pub start_time: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_time: Option<u64>,
    /// 16-byte UUID hex of the bound channel, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_attendees: Option<u32>,
    pub created_at: u64,
    /// `scheduled` / `active` / `completed` / `cancelled`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rsvps: Vec<EventRsvp>,
    /// Architecture §21 line 2624 — peer-cached cover image content
    /// hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_image_ref: Option<String>,
    /// Architecture §21 line 2628.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recurrence: Option<RecurrenceRule>,
    /// Architecture §21 line 2629.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<EventLocation>,
}

#[cfg(test)]
mod info_tests {
    use super::*;

    #[test]
    fn event_info_roundtrip() {
        let e = EventInfo {
            id: "evt_01".into(),
            title: "raid night".into(),
            description: "bring potions".into(),
            creator_pseudonym: "abcd".into(),
            start_time: 1_000,
            end_time: Some(2_000),
            channel_id: Some("ch_01".into()),
            max_attendees: Some(40),
            created_at: 500,
            status: "scheduled".into(),
            rsvps: vec![EventRsvp {
                pseudonym_key: "abcd".into(),
                status: "going".into(),
            }],
            cover_image_ref: Some("blake3:abcdef".into()),
            recurrence: None,
            location: None,
        };
        let json = serde_json::to_string(&e).unwrap();
        let back: EventInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
    }
}

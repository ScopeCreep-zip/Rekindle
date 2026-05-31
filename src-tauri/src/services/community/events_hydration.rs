//! Materialise `governance_state.events` into the SQLite
//! `community_events` table.
//!
//! Architecture §32 W16 — events are written to TWO places: the durable
//! governance entry (CRDT-merged) and a fast-path control envelope
//! (`ControlPayload::EventCreated`) that handles the live SQLite insert
//! at `services/veilid/control_event_records.rs`. Late joiners — who
//! only see the governance entries through the periodic DHT merge —
//! never observed the live envelope, so without this hydration their
//! `community_events` table stays empty and `get_events`,
//! `event_reminders`, and the calendar UI all see nothing despite the
//! events being durably present in governance.
//!
//! Called from `state_helpers::set_governance_state` after every CRDT
//! merge. `INSERT OR IGNORE` so this never clobbers richer fast-path
//! rows (e.g. `max_attendees` lives only in the gossip DTO and isn't
//! carried by the governance entry).

use std::sync::Arc;

use crate::db::DbPool;
use crate::db_helpers::db_fire;
use crate::state::AppState;
use crate::state_helpers;

pub fn hydrate_events_from_governance(state: &Arc<AppState>, pool: &DbPool, community_id: &str) {
    let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
    if owner_key.is_empty() {
        return;
    }
    let snapshot: Vec<EventRow> = {
        let communities = state.communities.read();
        let Some(community) = communities.get(community_id) else {
            return;
        };
        let Some(gov) = community.governance_state.as_ref() else {
            return;
        };
        gov.events
            .iter()
            .map(|(event_id, event)| EventRow {
                id: format!("evt_{}", hex::encode(event_id.0)),
                title: event.name.clone(),
                description: event.description.clone().unwrap_or_default(),
                creator_pseudonym: event
                    .creator_pseudonym
                    .as_ref()
                    .map(|p| hex::encode(p.0))
                    .unwrap_or_default(),
                start_time: i64::try_from(event.start_time).unwrap_or(i64::MAX),
                end_time: event.end_time.and_then(|t| i64::try_from(t).ok()),
                channel_id: event.channel_id.map(|c| hex::encode(c.0)),
                cover_image_ref: event.cover_image_ref.clone(),
                recurrence_json: event
                    .recurrence
                    .as_ref()
                    .and_then(|r| serde_json::to_string(r).ok()),
                location_json: event
                    .location
                    .as_ref()
                    .and_then(|l| serde_json::to_string(l).ok()),
                status: event_status_label(event.status),
            })
            .collect()
    };

    if snapshot.is_empty() {
        return;
    }

    let community_id = community_id.to_string();
    db_fire(pool, "hydrate events from governance", move |conn| {
        for row in snapshot {
            // Architecture §32 W16 — INSERT OR IGNORE preserves the
            // gossip-path row when it already exists (richer fields).
            conn.execute(
                "INSERT OR IGNORE INTO community_events \
                 (owner_key, community_id, id, title, description, creator_pseudonym, \
                  start_time, end_time, channel_id, max_attendees, created_at, status, \
                  cover_image_ref, recurrence_json, location_json) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,NULL,?10,?11,?12,?13,?14)",
                rusqlite::params![
                    owner_key,
                    community_id,
                    row.id,
                    row.title,
                    row.description,
                    row.creator_pseudonym,
                    row.start_time,
                    row.end_time,
                    row.channel_id,
                    // governance entries don't carry `created_at`; use
                    // `start_time` as a stable proxy for sort order.
                    row.start_time,
                    row.status,
                    row.cover_image_ref,
                    row.recurrence_json,
                    row.location_json,
                ],
            )?;
        }
        Ok(())
    });
}

struct EventRow {
    id: String,
    title: String,
    description: String,
    creator_pseudonym: String,
    start_time: i64,
    end_time: Option<i64>,
    channel_id: Option<String>,
    cover_image_ref: Option<String>,
    recurrence_json: Option<String>,
    location_json: Option<String>,
    status: String,
}

fn event_status_label(status: rekindle_types::event::EventStatus) -> String {
    match status {
        rekindle_types::event::EventStatus::Scheduled => "scheduled".into(),
        rekindle_types::event::EventStatus::Active => "active".into(),
        rekindle_types::event::EventStatus::Cancelled => "cancelled".into(),
        rekindle_types::event::EventStatus::Completed => "completed".into(),
    }
}

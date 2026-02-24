use std::sync::Arc;

use rekindle_protocol::dht::community::permissions;
use rekindle_protocol::messaging::envelope::{
    CommunityBroadcast, CommunityResponse, EventDto, EventRsvpDto,
};
use rusqlite::params;

use crate::audit;
use crate::server_state::ServerState;

use super::broadcast::broadcast_to_members;
use super::permissions::{check_permission, verify_membership};

pub(super) fn handle_create_event(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    title: &str,
    description: &str,
    start_time: u64,
    end_time: Option<u64>,
    channel_id: Option<&str>,
    max_attendees: Option<u32>,
) -> CommunityResponse {
    {
        let hosted = state.hosted.read();
        let Some(community) = hosted.get(community_id) else {
            return CommunityResponse::Error {
                code: 404,
                message: "community not found".into(),
            };
        };
        if let Err(e) = verify_membership(community, sender_pseudonym) {
            return e;
        }
        if let Err(e) =
            check_permission(community, sender_pseudonym, permissions::MANAGE_EVENTS)
        {
            return e;
        }
    }

    let now = rekindle_utils::timestamp_secs_i64();
    let event_id = format!("evt_{}", hex::encode(super::rand_bytes(8)));

    let start_i64 = i64::try_from(start_time).unwrap_or(i64::MAX);
    let end_i64 = end_time.map(|t| i64::try_from(t).unwrap_or(i64::MAX));
    let max_att_i64 = max_attendees.map(i64::from);

    let db = crate::db_helpers::lock_db(&state.db);
    if let Err(e) = db.execute(
        "INSERT INTO server_events (community_id, id, title, description, creator_pseudonym, start_time, end_time, channel_id, max_attendees, created_at, status) VALUES (?,?,?,?,?,?,?,?,?,?,'scheduled')",
        params![community_id, event_id, title, description, sender_pseudonym, start_i64, end_i64, channel_id, max_att_i64, now],
    ) {
        tracing::error!(error = %e, "failed to create event");
        return CommunityResponse::Error {
            code: 500,
            message: "failed to create event".into(),
        };
    }
    drop(db);

    audit::log_action(
        state,
        community_id,
        audit::AuditAction::CreateEvent,
        sender_pseudonym,
        Some(&event_id),
        Some(title),
    );

    let event = EventDto {
        id: event_id.clone(),
        title: title.to_string(),
        description: description.to_string(),
        creator_pseudonym: sender_pseudonym.to_string(),
        start_time,
        end_time,
        channel_id: channel_id.map(str::to_string),
        max_attendees,
        created_at: now.try_into().unwrap_or(0),
        status: "scheduled".to_string(),
        rsvps: vec![],
    };

    broadcast_to_members(
        state,
        community_id,
        "",
        &CommunityBroadcast::EventCreated {
            community_id: community_id.to_string(),
            event,
        },
    );

    CommunityResponse::EventCreated { event_id }
}

pub(super) fn handle_edit_event(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    event_id: &str,
    title: Option<&str>,
    description: Option<&str>,
    start_time: Option<u64>,
    end_time: Option<u64>,
    channel_id: Option<&str>,
    max_attendees: Option<u32>,
) -> CommunityResponse {
    {
        let hosted = state.hosted.read();
        let Some(community) = hosted.get(community_id) else {
            return CommunityResponse::Error {
                code: 404,
                message: "community not found".into(),
            };
        };
        if let Err(e) = verify_membership(community, sender_pseudonym) {
            return e;
        }
        if let Err(e) =
            check_permission(community, sender_pseudonym, permissions::MANAGE_EVENTS)
        {
            return e;
        }
    }

    let db = crate::db_helpers::lock_db(&state.db);

    // Build dynamic UPDATE query
    // Sentinel values clear nullable fields: end_time=0, channel_id="", max_attendees=0
    let mut updates = Vec::new();
    let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(t) = title {
        updates.push("title = ?");
        values.push(Box::new(t.to_string()));
    }
    if let Some(d) = description {
        updates.push("description = ?");
        values.push(Box::new(d.to_string()));
    }
    if let Some(st) = start_time {
        updates.push("start_time = ?");
        values.push(Box::new(i64::try_from(st).unwrap_or(i64::MAX)));
    }
    if let Some(et) = end_time {
        updates.push("end_time = ?");
        if et == 0 {
            values.push(Box::new(None::<i64>));
        } else {
            values.push(Box::new(Some(i64::try_from(et).unwrap_or(i64::MAX))));
        }
    }
    if let Some(ch) = channel_id {
        updates.push("channel_id = ?");
        if ch.is_empty() {
            values.push(Box::new(None::<String>));
        } else {
            values.push(Box::new(Some(ch.to_string())));
        }
    }
    if let Some(ma) = max_attendees {
        updates.push("max_attendees = ?");
        if ma == 0 {
            values.push(Box::new(None::<i64>));
        } else {
            values.push(Box::new(Some(i64::from(ma))));
        }
    }

    if updates.is_empty() {
        return CommunityResponse::Ok;
    }

    let sql = format!(
        "UPDATE server_events SET {} WHERE community_id = ? AND id = ?",
        updates.join(", ")
    );
    values.push(Box::new(community_id.to_string()));
    values.push(Box::new(event_id.to_string()));

    let params_ref: Vec<&dyn rusqlite::types::ToSql> =
        values.iter().map(AsRef::as_ref).collect();
    if let Err(e) = db.execute(&sql, params_ref.as_slice()) {
        tracing::error!(error = %e, "failed to edit event");
        return CommunityResponse::Error {
            code: 500,
            message: "failed to edit event".into(),
        };
    }

    // Fetch updated event for broadcast
    let event = load_event_dto(&db, community_id, event_id);
    drop(db);

    audit::log_action(
        state,
        community_id,
        audit::AuditAction::EditEvent,
        sender_pseudonym,
        Some(event_id),
        None,
    );

    if let Some(event) = event {
        broadcast_to_members(
            state,
            community_id,
            "",
            &CommunityBroadcast::EventUpdated {
                community_id: community_id.to_string(),
                event,
            },
        );
    }

    CommunityResponse::Ok
}

pub(super) fn handle_delete_event(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    event_id: &str,
) -> CommunityResponse {
    {
        let hosted = state.hosted.read();
        let Some(community) = hosted.get(community_id) else {
            return CommunityResponse::Error {
                code: 404,
                message: "community not found".into(),
            };
        };
        if let Err(e) = verify_membership(community, sender_pseudonym) {
            return e;
        }
    }

    // Allow deletion by the event creator or someone with MANAGE_EVENTS
    let db = crate::db_helpers::lock_db(&state.db);
    let creator: Option<String> = db
        .query_row(
            "SELECT creator_pseudonym FROM server_events WHERE community_id = ? AND id = ?",
            params![community_id, event_id],
            |row| row.get(0),
        )
        .ok();

    if creator.as_deref() != Some(sender_pseudonym) {
        let hosted = state.hosted.read();
        if let Some(community) = hosted.get(community_id) {
            if let Err(e) =
                check_permission(community, sender_pseudonym, permissions::MANAGE_EVENTS)
            {
                return e;
            }
        }
    }

    let _ = db.execute(
        "DELETE FROM server_event_rsvps WHERE community_id = ? AND event_id = ?",
        params![community_id, event_id],
    );
    let deleted = db
        .execute(
            "DELETE FROM server_events WHERE community_id = ? AND id = ?",
            params![community_id, event_id],
        )
        .unwrap_or(0);
    drop(db);

    if deleted > 0 {
        audit::log_action(
            state,
            community_id,
            audit::AuditAction::DeleteEvent,
            sender_pseudonym,
            Some(event_id),
            None,
        );

        broadcast_to_members(
            state,
            community_id,
            "",
            &CommunityBroadcast::EventDeleted {
                community_id: community_id.to_string(),
                event_id: event_id.to_string(),
            },
        );
    }

    CommunityResponse::Ok
}

pub(super) fn handle_cancel_event(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    event_id: &str,
) -> CommunityResponse {
    {
        let hosted = state.hosted.read();
        let Some(community) = hosted.get(community_id) else {
            return CommunityResponse::Error {
                code: 404,
                message: "community not found".into(),
            };
        };
        if let Err(e) = verify_membership(community, sender_pseudonym) {
            return e;
        }
    }

    // Allow cancellation by the event creator or someone with MANAGE_EVENTS
    let db = crate::db_helpers::lock_db(&state.db);
    let creator: Option<String> = db
        .query_row(
            "SELECT creator_pseudonym FROM server_events WHERE community_id = ? AND id = ?",
            params![community_id, event_id],
            |row| row.get(0),
        )
        .ok();

    if creator.as_deref() != Some(sender_pseudonym) {
        let hosted = state.hosted.read();
        if let Some(community) = hosted.get(community_id) {
            if let Err(e) =
                check_permission(community, sender_pseudonym, permissions::MANAGE_EVENTS)
            {
                return e;
            }
        }
    }

    let updated = db
        .execute(
            "UPDATE server_events SET status = 'canceled' WHERE community_id = ? AND id = ? AND status != 'canceled'",
            params![community_id, event_id],
        )
        .unwrap_or(0);

    if updated == 0 {
        return CommunityResponse::Error {
            code: 404,
            message: "event not found or already canceled".into(),
        };
    }

    let event = load_event_dto(&db, community_id, event_id);
    drop(db);

    audit::log_action(
        state,
        community_id,
        audit::AuditAction::CancelEvent,
        sender_pseudonym,
        Some(event_id),
        None,
    );

    if let Some(event) = event {
        broadcast_to_members(
            state,
            community_id,
            "",
            &CommunityBroadcast::EventUpdated {
                community_id: community_id.to_string(),
                event,
            },
        );
    }

    CommunityResponse::Ok
}

pub(super) fn handle_rsvp_event(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
    event_id: &str,
    status: &str,
) -> CommunityResponse {
    {
        let hosted = state.hosted.read();
        let Some(community) = hosted.get(community_id) else {
            return CommunityResponse::Error {
                code: 404,
                message: "community not found".into(),
            };
        };
        if let Err(e) = verify_membership(community, sender_pseudonym) {
            return e;
        }
    }

    // Validate status
    if !matches!(status, "going" | "maybe" | "declined") {
        return CommunityResponse::Error {
            code: 400,
            message: "invalid RSVP status (use going/maybe/declined)".into(),
        };
    }

    // Check max attendees if going
    let db = crate::db_helpers::lock_db(&state.db);

    if status == "going" {
        let max: Option<i64> = db
            .query_row(
                "SELECT max_attendees FROM server_events WHERE community_id = ? AND id = ?",
                params![community_id, event_id],
                |row| row.get(0),
            )
            .ok()
            .flatten();

        if let Some(max) = max {
            let going_count: i64 = db
                .query_row(
                    "SELECT COUNT(*) FROM server_event_rsvps WHERE community_id = ? AND event_id = ? AND status = 'going' AND pseudonym_key_hex != ?",
                    params![community_id, event_id, sender_pseudonym],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            if going_count >= max {
                return CommunityResponse::Error {
                    code: 409,
                    message: "event is full".into(),
                };
            }
        }
    }

    if let Err(e) = db.execute(
        "INSERT OR REPLACE INTO server_event_rsvps (community_id, event_id, pseudonym_key_hex, status) VALUES (?,?,?,?)",
        params![community_id, event_id, sender_pseudonym, status],
    ) {
        tracing::error!(error = %e, "failed to save RSVP");
        return CommunityResponse::Error {
            code: 500,
            message: "failed to save RSVP".into(),
        };
    }
    drop(db);

    broadcast_to_members(
        state,
        community_id,
        "",
        &CommunityBroadcast::EventRsvpChanged {
            community_id: community_id.to_string(),
            event_id: event_id.to_string(),
            pseudonym_key: sender_pseudonym.to_string(),
            status: status.to_string(),
        },
    );

    CommunityResponse::Ok
}

pub(super) fn handle_get_events(
    state: &Arc<ServerState>,
    community_id: &str,
    sender_pseudonym: &str,
) -> CommunityResponse {
    {
        let hosted = state.hosted.read();
        let Some(community) = hosted.get(community_id) else {
            return CommunityResponse::Error {
                code: 404,
                message: "community not found".into(),
            };
        };
        if let Err(e) = verify_membership(community, sender_pseudonym) {
            return e;
        }
    }

    let db = crate::db_helpers::lock_db(&state.db);
    let events = load_all_events(&db, community_id);
    CommunityResponse::EventList { events }
}

/// Load a single event with its RSVPs.
fn load_event_dto(
    db: &rusqlite::Connection,
    community_id: &str,
    event_id: &str,
) -> Option<EventDto> {
    let row = db
        .query_row(
            "SELECT id, title, description, creator_pseudonym, start_time, end_time, channel_id, max_attendees, created_at, status \
             FROM server_events WHERE community_id = ? AND id = ?",
            params![community_id, event_id],
            |row| {
                Ok(EventDto {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    description: row.get(2)?,
                    creator_pseudonym: row.get(3)?,
                    start_time: row.get::<_, i64>(4).map(i64::cast_unsigned)?,
                    end_time: row
                        .get::<_, Option<i64>>(5)
                        .map(|v| v.map(i64::cast_unsigned))?,
                    channel_id: row.get(6)?,
                    max_attendees: row
                        .get::<_, Option<i64>>(7)
                        .map(|v| v.and_then(|n| u32::try_from(n).ok()))?,
                    created_at: row.get::<_, i64>(8).map(i64::cast_unsigned)?,
                    status: row.get(9)?,
                    rsvps: vec![],
                })
            },
        )
        .ok()?;

    let rsvps = load_event_rsvps(db, community_id, event_id);
    Some(EventDto { rsvps, ..row })
}

/// Load all events for a community with RSVPs.
pub(super) fn load_all_events(db: &rusqlite::Connection, community_id: &str) -> Vec<EventDto> {
    let Ok(mut stmt) = db.prepare(
        "SELECT id, title, description, creator_pseudonym, start_time, end_time, channel_id, max_attendees, created_at, status \
         FROM server_events WHERE community_id = ? ORDER BY start_time ASC",
    ) else {
        tracing::error!("failed to prepare events query");
        return vec![];
    };

    let events: Vec<EventDto> = stmt
        .query_map(params![community_id], |row| {
            Ok(EventDto {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get(2)?,
                creator_pseudonym: row.get(3)?,
                start_time: row.get::<_, i64>(4).map(i64::cast_unsigned)?,
                end_time: row
                    .get::<_, Option<i64>>(5)
                    .map(|v| v.map(i64::cast_unsigned))?,
                channel_id: row.get(6)?,
                max_attendees: row
                    .get::<_, Option<i64>>(7)
                    .map(|v| v.and_then(|n| u32::try_from(n).ok()))?,
                created_at: row.get::<_, i64>(8).map(i64::cast_unsigned)?,
                status: row.get(9)?,
                rsvps: vec![],
            })
        })
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default();

    events
        .into_iter()
        .map(|mut evt| {
            evt.rsvps = load_event_rsvps(db, community_id, &evt.id);
            evt
        })
        .collect()
}

pub(super) fn load_event_rsvps(
    db: &rusqlite::Connection,
    community_id: &str,
    event_id: &str,
) -> Vec<EventRsvpDto> {
    let Ok(mut stmt) = db.prepare(
        "SELECT pseudonym_key_hex, status FROM server_event_rsvps WHERE community_id = ? AND event_id = ?",
    ) else {
        return vec![];
    };
    stmt.query_map(params![community_id, event_id], |row| {
        Ok(EventRsvpDto {
            pseudonym_key: row.get(0)?,
            status: row.get(1)?,
        })
    })
    .map(|rows| rows.flatten().collect())
    .unwrap_or_default()
}

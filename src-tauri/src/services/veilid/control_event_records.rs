use std::sync::Arc;

use crate::db::DbPool;
use crate::state::AppState;
use crate::state_helpers;

pub(super) fn handle_event_payload(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    payload: rekindle_protocol::dht::community::envelope::ControlPayload,
) {
    use crate::channels::CommunityEvent;
    use rekindle_protocol::dht::community::envelope::ControlPayload;

    match payload {
        ControlPayload::EventCreated { event } => {
            handle_event_upsert(app_handle, state, pool, community_id, event, true);
            crate::services::community::wake_event_reminders(state);
        }
        ControlPayload::EventUpdated { event } => {
            handle_event_upsert(app_handle, state, pool, community_id, event, false);
            crate::services::community::wake_event_reminders(state);
        }
        ControlPayload::EventDeleted { event_id } => {
            let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
            let cid = community_id.to_string();
            let eid = event_id.clone();
            crate::db_helpers::db_fire(pool, "delete event", move |conn| {
                conn.execute(
                    "DELETE FROM community_events WHERE owner_key = ?1 AND community_id = ?2 AND id = ?3",
                    rusqlite::params![owner_key, cid, eid],
                )?;
                conn.execute(
                    "DELETE FROM event_rsvps WHERE owner_key = ?1 AND community_id = ?2 AND event_id = ?3",
                    rusqlite::params![owner_key, cid, eid],
                )?;
                Ok(())
            });
            crate::event_dispatch::emit_live(
                app_handle,
                "community-event",
                &CommunityEvent::EventDeleted {
                    community_id: community_id.to_string(),
                    event_id,
                },
            );
            crate::services::community::wake_event_reminders(state);
        }
        ControlPayload::EventRsvpChanged {
            event_id,
            pseudonym_key,
            status,
        } => {
            let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
            let cid = community_id.to_string();
            let eid = event_id.clone();
            let pk = pseudonym_key.clone();
            let st = status.clone();
            crate::db_helpers::db_fire(pool, "persist rsvp", move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO event_rsvps \
                     (owner_key, community_id, event_id, pseudonym_key, status) \
                     VALUES (?1,?2,?3,?4,?5)",
                    rusqlite::params![owner_key, cid, eid, pk, st],
                )?;
                Ok(())
            });
            crate::event_dispatch::emit_live(
                app_handle,
                "community-event",
                &CommunityEvent::EventRsvpChanged {
                    community_id: community_id.to_string(),
                    event_id,
                    pseudonym_key,
                    status,
                },
            );
        }
        _ => {}
    }
}

pub(super) fn handle_game_server_payload(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    payload: rekindle_protocol::dht::community::envelope::ControlPayload,
) {
    use crate::channels::CommunityEvent;
    use rekindle_protocol::dht::community::envelope::ControlPayload;

    match payload {
        ControlPayload::GameServerAdded { server } => {
            let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
            let cid = community_id.to_string();
            let persisted = server.clone();
            crate::db_helpers::db_fire(pool, "persist game server", move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO game_servers \
                     (owner_key, community_id, id, game_id, label, address, added_by, created_at) \
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                    rusqlite::params![
                        owner_key,
                        cid,
                        persisted.id,
                        persisted.game_id,
                        persisted.label,
                        persisted.address,
                        persisted.added_by,
                        persisted.created_at,
                    ],
                )?;
                Ok(())
            });
            crate::event_dispatch::emit_live(
                app_handle,
                "community-event",
                &CommunityEvent::GameServerAdded {
                    community_id: community_id.to_string(),
                    server,
                },
            );
        }
        ControlPayload::GameServerRemoved { server_id } => {
            let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
            let cid = community_id.to_string();
            let sid = server_id.clone();
            crate::db_helpers::db_fire(pool, "remove game server", move |conn| {
                conn.execute(
                    "DELETE FROM game_servers WHERE owner_key = ?1 AND community_id = ?2 AND id = ?3",
                    rusqlite::params![owner_key, cid, sid],
                )?;
                Ok(())
            });
            crate::event_dispatch::emit_live(
                app_handle,
                "community-event",
                &CommunityEvent::GameServerRemoved {
                    community_id: community_id.to_string(),
                    server_id,
                },
            );
        }
        _ => {}
    }
}

fn handle_event_upsert(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    community_id: &str,
    event: rekindle_types::event::EventInfo,
    created: bool,
) {
    use crate::channels::CommunityEvent;

    let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
    let cid = community_id.to_string();
    let d = event.clone();
    let recurrence_json = d
        .recurrence
        .as_ref()
        .and_then(|r| serde_json::to_string(r).ok());
    let location_json = d
        .location
        .as_ref()
        .and_then(|l| serde_json::to_string(l).ok());
    let cover_image_ref = d.cover_image_ref.clone();
    crate::db_helpers::db_fire(pool, "persist event", move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO community_events \
             (owner_key, community_id, id, title, description, creator_pseudonym, \
              start_time, end_time, channel_id, max_attendees, created_at, status, \
              cover_image_ref, recurrence_json, location_json) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
            rusqlite::params![
                owner_key,
                cid,
                d.id,
                d.title,
                d.description,
                d.creator_pseudonym,
                d.start_time,
                d.end_time,
                d.channel_id,
                d.max_attendees,
                d.created_at,
                d.status,
                cover_image_ref,
                recurrence_json,
                location_json,
            ],
        )?;
        Ok(())
    });
    let event_kind = if created {
        CommunityEvent::EventCreated {
            community_id: community_id.to_string(),
            event,
        }
    } else {
        CommunityEvent::EventUpdated {
            community_id: community_id.to_string(),
            event,
        }
    };
    crate::event_dispatch::emit_live(app_handle, "community-event", &event_kind);
}

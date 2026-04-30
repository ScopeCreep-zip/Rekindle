use tauri::{Emitter, State};

use super::helpers::{random_nonce, require_permission};
use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_protocol::dht::community::permissions_v2::Permissions;

pub use crate::channels::community_channel::EventInfoDto;
pub use crate::channels::community_channel::EventRsvpInfoDto;

fn normalize_rsvp_status(status: &str) -> String {
    match status {
        "maybe" => "interested".to_string(),
        "going" | "interested" | "declined" => status.to_string(),
        _ => "declined".to_string(),
    }
}

#[tauri::command]
pub async fn create_event(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    title: String,
    description: String,
    start_time: u64,
    end_time: Option<u64>,
    channel_id: Option<String>,
    max_attendees: Option<u32>,
) -> Result<String, String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_EVENTS)?;
    let _ = pool;
    let event_id = format!("evt_{}", hex::encode(random_nonce(8)));

    let creator_pseudonym = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_default()
    };
    let event = EventInfoDto {
        id: event_id.clone(),
        title,
        description,
        creator_pseudonym,
        start_time,
        end_time,
        channel_id,
        max_attendees,
        created_at: rekindle_utils::timestamp_secs(),
        status: "scheduled".to_string(),
        rsvps: Vec::new(),
    };

    crate::services::community::send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::EventCreated {
            event: serde_json::to_value(&event).map_err(|e| format!("serialize event: {e}"))?,
        }),
    )?;

    Ok(event_id)
}

#[tauri::command]
pub async fn edit_event(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    event_id: String,
    title: Option<String>,
    description: Option<String>,
    start_time: Option<u64>,
    end_time: Option<u64>,
    channel_id: Option<String>,
    max_attendees: Option<u32>,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_EVENTS)?;
    let events = get_events(state.clone(), pool.clone(), community_id.clone()).await?;
    let Some(existing) = events.into_iter().find(|event| event.id == event_id) else {
        return Err("event not found".into());
    };
    let event = EventInfoDto {
        id: existing.id,
        title: title.unwrap_or(existing.title),
        description: description.unwrap_or(existing.description),
        creator_pseudonym: existing.creator_pseudonym,
        start_time: start_time.unwrap_or(existing.start_time),
        end_time: end_time.or(existing.end_time),
        channel_id: channel_id.or(existing.channel_id),
        max_attendees: max_attendees.or(existing.max_attendees),
        created_at: existing.created_at,
        status: existing.status,
        rsvps: existing.rsvps,
    };
    crate::services::community::send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::EventUpdated {
            event: serde_json::to_value(&event).map_err(|e| format!("serialize event: {e}"))?,
        }),
    )
}

#[tauri::command]
pub async fn delete_event(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    event_id: String,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_EVENTS)?;
    let _ = pool;
    crate::services::community::send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::EventDeleted { event_id }),
    )
}

#[tauri::command]
pub async fn cancel_event(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    event_id: String,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::MANAGE_EVENTS)?;
    let events = get_events(state.clone(), pool.clone(), community_id.clone()).await?;
    let Some(existing) = events.into_iter().find(|event| event.id == event_id) else {
        return Err("event not found".into());
    };
    let event = EventInfoDto {
        status: "canceled".to_string(),
        ..existing
    };
    crate::services::community::send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::EventUpdated {
            event: serde_json::to_value(&event).map_err(|e| format!("serialize event: {e}"))?,
        }),
    )
}

#[tauri::command]
pub async fn rsvp_event(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    event_id: String,
    status: String,
) -> Result<(), String> {
    set_event_rsvp(state, pool, community_id, event_id, status).await
}

#[tauri::command]
pub async fn set_event_rsvp(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    event_id: String,
    status: String,
) -> Result<(), String> {
    require_permission(state.inner(), &community_id, Permissions::VIEW_CHANNEL)?;
    let normalized_status = normalize_rsvp_status(&status);
    let pseudonym_key = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_default()
    };
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let community_id_for_db = community_id.clone();
    let event_id_for_db = event_id.clone();
    let status_for_db = normalized_status.clone();
    db_call(pool.inner(), move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO community_event_rsvps \
             (owner_key, community_id, event_id, status) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                owner_key,
                community_id_for_db,
                event_id_for_db,
                status_for_db
            ],
        )?;
        Ok(())
    })
    .await?;
    {
        let mut communities = state.communities.write();
        if let Some(community) = communities.get_mut(&community_id) {
            community
                .my_event_rsvps
                .insert(event_id.clone(), normalized_status.clone());
            let rsvps = community
                .event_rsvps_by_event
                .entry(event_id.clone())
                .or_default();
            if let Some(existing) = rsvps
                .iter_mut()
                .find(|entry| entry.pseudonym_key == pseudonym_key)
            {
                existing.status.clone_from(&normalized_status);
            } else {
                rsvps.push(crate::state::EventRsvpEntry {
                    pseudonym_key: pseudonym_key.clone(),
                    status: normalized_status.clone(),
                });
            }
        }
    }
    crate::services::community::send_to_mesh(
        state.inner(),
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::EventRsvpChanged {
            event_id: event_id.clone(),
            pseudonym_key: pseudonym_key.clone(),
            status: normalized_status.clone(),
        }),
    )?;
    if let Some(app_handle) = state_helpers::app_handle(state.inner()) {
        let _ = app_handle.emit(
            "community-event",
            crate::channels::CommunityEvent::EventRsvpChanged {
                community_id: community_id.clone(),
                event_id: event_id.clone(),
                pseudonym_key,
                status: normalized_status,
            },
        );
    }
    let state_clone = state.inner().clone();
    let community_id_clone = community_id.clone();
    tauri::async_runtime::spawn(async move {
        let _ = crate::services::community::presence_poll_tick_public(
            &state_clone,
            &community_id_clone,
        )
        .await;
    });
    Ok(())
}

#[tauri::command]
pub async fn list_event_attendees(
    state: State<'_, SharedState>,
    community_id: String,
    event_id: String,
) -> Result<Vec<EventRsvpInfoDto>, String> {
    require_permission(state.inner(), &community_id, Permissions::VIEW_CHANNEL)?;
    let attendees = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|community| community.event_rsvps_by_event.get(&event_id).cloned())
            .unwrap_or_default()
    };
    Ok(attendees
        .into_iter()
        .map(|entry| EventRsvpInfoDto {
            pseudonym_key: entry.pseudonym_key,
            status: entry.status,
        })
        .collect())
}

#[tauri::command]
pub async fn get_events(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
) -> Result<Vec<EventInfoDto>, String> {
    require_permission(state.inner(), &community_id, Permissions::VIEW_CHANNEL)?;
    let owner_key = state_helpers::current_owner_key(state.inner())?;
    let community_id_for_db = community_id.clone();
    let mut events: Vec<EventInfoDto> = db_call(pool.inner(), move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, title, description, creator_pseudonym, start_time, end_time, \
                    channel_id, max_attendees, created_at, status \
             FROM community_events \
             WHERE owner_key = ?1 AND community_id = ?2 \
             ORDER BY start_time ASC",
        )?;
        let events: Vec<EventInfoDto> = stmt
            .query_map(rusqlite::params![owner_key, community_id_for_db], |row| {
                Ok(EventInfoDto {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    description: row.get(2)?,
                    creator_pseudonym: row.get(3)?,
                    start_time: row.get::<_, i64>(4).unwrap_or(0).cast_unsigned(),
                    end_time: row.get::<_, Option<i64>>(5)?.map(i64::cast_unsigned),
                    channel_id: row.get(6)?,
                    max_attendees: row.get::<_, Option<i32>>(7)?.map(i32::cast_unsigned),
                    created_at: row.get::<_, i64>(8).unwrap_or(0).cast_unsigned(),
                    status: row.get(9)?,
                    rsvps: Vec::new(),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(events)
    })
    .await?;

    let (aggregated_rsvps, my_event_rsvps, my_pseudonym_key) = {
        let communities = state.communities.read();
        let community = communities.get(&community_id);
        (
            community
                .map(|community| community.event_rsvps_by_event.clone())
                .unwrap_or_default(),
            community
                .map(|community| community.my_event_rsvps.clone())
                .unwrap_or_default(),
            community.and_then(|community| community.my_pseudonym_key.clone()),
        )
    };

    for event in &mut events {
        if let Some(entries) = aggregated_rsvps.get(&event.id) {
            event.rsvps = entries
                .iter()
                .cloned()
                .map(|entry| EventRsvpInfoDto {
                    pseudonym_key: entry.pseudonym_key,
                    status: entry.status,
                })
                .collect();
        } else if let (Some(status), Some(pseudonym_key)) = (
            my_event_rsvps.get(&event.id).cloned(),
            my_pseudonym_key.clone(),
        ) {
            event.rsvps = vec![EventRsvpInfoDto {
                pseudonym_key,
                status,
            }];
        }
    }

    Ok(events)
}

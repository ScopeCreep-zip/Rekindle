//! Phase 23.C — event-creation runtime orchestration lifted from
//! `commands/community/events.rs`. Same pattern as the sibling
//! `community_*_runtime.rs` modules.

use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_protocol::dht::community::permissions_v2::Permissions;

use crate::channels::community_channel::EventInfoDto;
use crate::db::DbPool;
use crate::state::SharedState;
use crate::state_helpers;

pub fn parse_event_id(id: &str) -> rekindle_types::id::EventId {
    let stripped = id.strip_prefix("evt_").unwrap_or(id);
    let mut buf = [0u8; 16];
    if let Ok(decoded) = hex::decode(stripped) {
        let len = decoded.len().min(16);
        buf[..len].copy_from_slice(&decoded[..len]);
    }
    rekindle_types::id::EventId(buf)
}

pub fn parse_channel_id(id: &str) -> rekindle_types::id::ChannelId {
    let mut buf = [0u8; 16];
    if let Ok(decoded) = hex::decode(id) {
        let len = decoded.len().min(16);
        buf[..len].copy_from_slice(&decoded[..len]);
    }
    rekindle_types::id::ChannelId(buf)
}

pub fn parse_pseudonym(hex_str: &str) -> Option<rekindle_types::id::PseudonymKey> {
    let bytes = hex::decode(hex_str).ok()?;
    let arr: [u8; 32] = bytes.as_slice().try_into().ok()?;
    Some(rekindle_types::id::PseudonymKey(arr))
}

pub fn normalize_rsvp_status(status: &str) -> String {
    match status {
        "maybe" => "interested".to_string(),
        "going" | "interested" | "declined" => status.to_string(),
        _ => "declined".to_string(),
    }
}

/// Tauri-runtime orchestration: validate title/description sizes,
/// generate event_id, write governance `EventCreated` entry, broadcast
/// `ControlPayload::EventCreated`. Returns the new event_id.
pub async fn create_event_inner(
    state: &SharedState,
    community_id: &str,
    title: String,
    description: String,
    start_time: u64,
    end_time: Option<u64>,
    channel_id: Option<String>,
    max_attendees: Option<u32>,
    cover_image_ref: Option<String>,
    recurrence: Option<rekindle_types::event::RecurrenceRule>,
    location: Option<rekindle_types::event::EventLocation>,
) -> Result<String, String> {
    use crate::commands::community::helpers::{random_nonce, require_permission};

    require_permission(state, community_id, Permissions::MANAGE_EVENTS)?;
    if title.chars().count() > rekindle_types::event::MAX_EVENT_NAME_CHARS {
        return Err(format!(
            "event name exceeds {} characters (architecture §21)",
            rekindle_types::event::MAX_EVENT_NAME_CHARS
        ));
    }
    if description.chars().count() > rekindle_types::event::MAX_EVENT_DESCRIPTION_CHARS {
        return Err(format!(
            "event description exceeds {} characters (architecture §21)",
            rekindle_types::event::MAX_EVENT_DESCRIPTION_CHARS
        ));
    }
    let event_id = format!("evt_{}", hex::encode(random_nonce(8)));

    let creator_pseudonym = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_default()
    };
    let event = EventInfoDto {
        id: event_id.clone(),
        title: title.clone(),
        description: description.clone(),
        creator_pseudonym: creator_pseudonym.clone(),
        start_time,
        end_time,
        channel_id: channel_id.clone(),
        max_attendees,
        created_at: rekindle_utils::timestamp_secs(),
        status: "scheduled".to_string(),
        rsvps: Vec::new(),
        cover_image_ref: cover_image_ref.clone(),
        recurrence: recurrence.clone(),
        location: location.clone(),
    };

    let lamport = state_helpers::increment_lamport(state, community_id);
    let governance_event_id = parse_event_id(&event_id);
    crate::services::community::write_entry(
        state,
        community_id,
        rekindle_types::governance::GovernanceEntry::EventCreated {
            event_id: governance_event_id,
            name: title,
            description: Some(description),
            start_time,
            end_time,
            channel_id: channel_id.as_deref().map(parse_channel_id),
            cover_image_ref,
            creator_pseudonym: parse_pseudonym(&creator_pseudonym),
            recurrence,
            location,
            status: Some(rekindle_types::event::EventStatus::Scheduled),
            lamport,
        },
    )
    .await?;

    crate::services::community::send_to_mesh(
        state,
        community_id,
        &CommunityEnvelope::Control(ControlPayload::EventCreated {
            event: event.clone(),
        }),
    )?;

    Ok(event_id)
}

pub async fn edit_event_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: String,
    event_id: String,
    title: Option<String>,
    description: Option<String>,
    start_time: Option<u64>,
    end_time: Option<u64>,
    channel_id: Option<String>,
    max_attendees: Option<u32>,
) -> Result<(), String> {
    use crate::commands::community::helpers::require_permission;

    require_permission(state, &community_id, Permissions::MANAGE_EVENTS)?;
    let events = get_events_inner(state, pool, community_id.clone()).await?;
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
        cover_image_ref: existing.cover_image_ref,
        recurrence: existing.recurrence,
        location: existing.location,
    };
    crate::services::community::send_to_mesh(
        state,
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::EventUpdated {
            event: event.clone(),
        }),
    )
}

pub fn delete_event_inner(
    state: &SharedState,
    community_id: &str,
    event_id: String,
) -> Result<(), String> {
    use crate::commands::community::helpers::require_permission;

    require_permission(state, community_id, Permissions::MANAGE_EVENTS)?;
    crate::services::community::send_to_mesh(
        state,
        community_id,
        &CommunityEnvelope::Control(ControlPayload::EventDeleted { event_id }),
    )
}

pub async fn cancel_event_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: String,
    event_id: String,
) -> Result<(), String> {
    use crate::commands::community::helpers::require_permission;

    require_permission(state, &community_id, Permissions::MANAGE_EVENTS)?;
    let events = get_events_inner(state, pool, community_id.clone()).await?;
    let Some(existing) = events.into_iter().find(|event| event.id == event_id) else {
        return Err("event not found".into());
    };
    let event = EventInfoDto {
        status: "cancelled".to_string(),
        ..existing
    };
    crate::services::community::send_to_mesh(
        state,
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::EventUpdated {
            event: event.clone(),
        }),
    )
}

pub async fn set_event_rsvp_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: String,
    event_id: String,
    status: String,
) -> Result<(), String> {
    use crate::commands::community::helpers::require_permission;
    use crate::db_helpers::db_call;

    require_permission(state, &community_id, Permissions::VIEW_CHANNEL)?;
    let normalized_status = normalize_rsvp_status(&status);
    let pseudonym_key = {
        let communities = state.communities.read();
        communities
            .get(&community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_default()
    };
    let owner_key = crate::state_helpers::current_owner_key(state)?;
    let community_id_for_db = community_id.clone();
    let event_id_for_db = event_id.clone();
    let status_for_db = normalized_status.clone();
    db_call(pool, move |conn| {
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
        state,
        &community_id,
        &CommunityEnvelope::Control(ControlPayload::EventRsvpChanged {
            event_id: event_id.clone(),
            pseudonym_key: pseudonym_key.clone(),
            status: normalized_status.clone(),
        }),
    )?;
    if let Some(app_handle) = crate::state_helpers::app_handle(state) {
        crate::event_dispatch::emit_live(
            &app_handle,
            "community-event",
            &crate::channels::CommunityEvent::EventRsvpChanged {
                community_id: community_id.clone(),
                event_id: event_id.clone(),
                pseudonym_key,
                status: normalized_status,
            },
        );
    }
    let state_clone = state.clone();
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

pub async fn get_events_inner(
    state: &SharedState,
    pool: &DbPool,
    community_id: String,
) -> Result<Vec<EventInfoDto>, String> {
    use crate::channels::community_channel::EventRsvpInfoDto;
    use crate::commands::community::helpers::require_permission;
    use crate::db_helpers::db_call;

    require_permission(state, &community_id, Permissions::VIEW_CHANNEL)?;
    let owner_key = crate::state_helpers::current_owner_key(state)?;
    let community_id_for_db = community_id.clone();
    let mut events: Vec<EventInfoDto> = db_call(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, title, description, creator_pseudonym, start_time, end_time, \
                    channel_id, max_attendees, created_at, status, \
                    cover_image_ref, recurrence_json, location_json \
             FROM community_events \
             WHERE owner_key = ?1 AND community_id = ?2 \
             ORDER BY start_time ASC",
        )?;
        let events: Vec<EventInfoDto> = stmt
            .query_map(rusqlite::params![owner_key, community_id_for_db], |row| {
                let recurrence_json: Option<String> = row.get(11).ok();
                let location_json: Option<String> = row.get(12).ok();
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
                    cover_image_ref: row.get(10).ok(),
                    recurrence: recurrence_json
                        .as_deref()
                        .and_then(|s| serde_json::from_str(s).ok()),
                    location: location_json
                        .as_deref()
                        .and_then(|s| serde_json::from_str(s).ok()),
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

pub fn list_event_attendees_inner(
    state: &SharedState,
    community_id: &str,
    event_id: &str,
) -> Result<Vec<crate::channels::community_channel::EventRsvpInfoDto>, String> {
    use crate::channels::community_channel::EventRsvpInfoDto;
    use crate::commands::community::helpers::require_permission;

    require_permission(state, community_id, Permissions::VIEW_CHANNEL)?;
    let attendees = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|community| community.event_rsvps_by_event.get(event_id).cloned())
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

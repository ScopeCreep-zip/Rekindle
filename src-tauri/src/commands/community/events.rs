use tauri::State;

use crate::db::DbPool;
use crate::services::community_event_runtime::{
    cancel_event_inner, delete_event_inner, edit_event_inner, get_events_inner,
    set_event_rsvp_inner,
};
use crate::state::SharedState;

pub use crate::channels::community_channel::EventInfoDto;
pub use crate::channels::community_channel::EventRsvpInfoDto;

#[tauri::command]
pub async fn create_event(
    state: State<'_, SharedState>,
    _pool: State<'_, DbPool>,
    community_id: String,
    request: CreateEventRequest,
) -> Result<String, String> {
    crate::services::community_event_runtime::create_event_inner(
        state.inner(),
        &community_id,
        request.title,
        request.description,
        request.start_time,
        request.end_time,
        request.channel_id,
        request.max_attendees,
        request.cover_image_ref,
        request.recurrence,
        request.location,
    )
    .await
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateEventRequest {
    pub title: String,
    pub description: String,
    pub start_time: u64,
    pub end_time: Option<u64>,
    pub channel_id: Option<String>,
    pub max_attendees: Option<u32>,
    /// Architecture §21 line 2624 — peer-cached cover image hex hash.
    pub cover_image_ref: Option<String>,
    /// Architecture §21 line 2628 — recurrence rule (None = one-off).
    pub recurrence: Option<rekindle_types::event::RecurrenceRule>,
    /// Architecture §21 line 2629 — event location.
    pub location: Option<rekindle_types::event::EventLocation>,
}


#[tauri::command]
#[allow(clippy::too_many_arguments, reason = "Tauri command surface — matches edit_event partial-update payload")]
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
    edit_event_inner(
        state.inner(),
        pool.inner(),
        community_id,
        event_id,
        title,
        description,
        start_time,
        end_time,
        channel_id,
        max_attendees,
    )
    .await
}

#[tauri::command]
pub async fn delete_event(
    state: State<'_, SharedState>,
    community_id: String,
    event_id: String,
) -> Result<(), String> {
    delete_event_inner(state.inner(), &community_id, event_id)
}

#[tauri::command]
pub async fn cancel_event(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
    event_id: String,
) -> Result<(), String> {
    cancel_event_inner(state.inner(), pool.inner(), community_id, event_id).await
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
    set_event_rsvp_inner(state.inner(), pool.inner(), community_id, event_id, status).await
}

#[tauri::command]
pub async fn list_event_attendees(
    state: State<'_, SharedState>,
    community_id: String,
    event_id: String,
) -> Result<Vec<EventRsvpInfoDto>, String> {
    crate::services::community_event_runtime::list_event_attendees_inner(
        state.inner(),
        &community_id,
        &event_id,
    )
}

#[tauri::command]
pub async fn get_events(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
    community_id: String,
) -> Result<Vec<EventInfoDto>, String> {
    get_events_inner(state.inner(), pool.inner(), community_id).await
}

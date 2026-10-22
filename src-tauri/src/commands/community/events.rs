//! A community's scheduled events and RSVPs (plan E3.1 E4): Host calls.

use rekindle_ipc::protocol::ActivityRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::{CommandError, Done};
use rekindle_types::event::{EventEdit, EventInfo, EventRsvp, NewEvent};
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

#[tauri::command]
pub async fn create_event(
    state: State<'_, SharedState>,
    community_id: String,
    request: NewEvent,
) -> Result<Done<String>, CommandError> {
    let request =
        IpcRequest::Community(CommunityRequest::Activities(ActivityRequest::EventCreate {
            community: community_id,
            event: request,
        }));
    host_call::call(&state, request).await
}

#[tauri::command]
pub async fn edit_event(
    state: State<'_, SharedState>,
    community_id: String,
    event_id: String,
    title: Option<String>,
    description: Option<String>,
    start_time: Option<u64>,
    end_time: Option<u64>,
    channel_id: Option<String>,
    max_attendees: Option<u32>,
) -> Result<Done<()>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::Activities(ActivityRequest::EventEdit {
        community: community_id,
        event_id,
        edit: EventEdit {
            title,
            description,
            start_time,
            end_time,
            channel_id,
            max_attendees,
        },
    }));
    host_call::call(&state, request).await
}

#[tauri::command]
pub async fn delete_event(
    state: State<'_, SharedState>,
    community_id: String,
    event_id: String,
) -> Result<Done<()>, CommandError> {
    let request =
        IpcRequest::Community(CommunityRequest::Activities(ActivityRequest::EventDelete {
            community: community_id,
            event_id,
        }));
    host_call::call(&state, request).await
}

#[tauri::command]
pub async fn cancel_event(
    state: State<'_, SharedState>,
    community_id: String,
    event_id: String,
) -> Result<Done<()>, CommandError> {
    let request =
        IpcRequest::Community(CommunityRequest::Activities(ActivityRequest::EventCancel {
            community: community_id,
            event_id,
        }));
    host_call::call(&state, request).await
}

#[tauri::command]
pub async fn rsvp_event(
    state: State<'_, SharedState>,
    community_id: String,
    event_id: String,
    status: String,
) -> Result<Done<()>, CommandError> {
    set_event_rsvp(state, community_id, event_id, status).await
}

#[tauri::command]
pub async fn set_event_rsvp(
    state: State<'_, SharedState>,
    community_id: String,
    event_id: String,
    status: String,
) -> Result<Done<()>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::Activities(ActivityRequest::EventRsvp {
        community: community_id,
        event_id,
        status,
    }));
    host_call::call(&state, request).await
}

#[tauri::command]
pub async fn list_event_attendees(
    state: State<'_, SharedState>,
    community_id: String,
    event_id: String,
) -> Result<Done<Vec<EventRsvp>>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::Activities(
        ActivityRequest::EventAttendees {
            community: community_id,
            event_id,
        },
    ));
    host_call::call(&state, request).await
}

#[tauri::command]
pub async fn get_events(
    state: State<'_, SharedState>,
    community_id: String,
) -> Result<Done<Vec<EventInfo>>, CommandError> {
    let request = IpcRequest::Community(CommunityRequest::Activities(ActivityRequest::EventList {
        community: community_id,
    }));
    host_call::call(&state, request).await
}

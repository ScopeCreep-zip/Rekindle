//! A community's scheduled events and RSVPs (plan E3.1 E4): Host calls.

use rekindle_ipc::protocol::ActivityRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::{CommandError, Done};
use rekindle_types::event::{EventEdit, EventInfo, EventRsvp, NewEvent};
use tauri::State;

use crate::state::SharedState;

host_command! {
    state;
    pub async fn create_event(
        state: State<'_, SharedState>,
        community_id: String,
        request: NewEvent,
    ) -> Result<Done<String>, CommandError> {
        IpcRequest::Community(CommunityRequest::Activities(ActivityRequest::EventCreate {
            community: community_id,
            event: request,
        }))
    }
}

host_command! {
    state;
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
        IpcRequest::Community(CommunityRequest::Activities(ActivityRequest::EventEdit {
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
        }))
    }
}

host_command! {
    state;
    pub async fn delete_event(
        state: State<'_, SharedState>,
        community_id: String,
        event_id: String,
    ) -> Result<Done<()>, CommandError> {
        IpcRequest::Community(CommunityRequest::Activities(ActivityRequest::EventDelete {
            community: community_id,
            event_id,
        }))
    }
}

host_command! {
    state;
    pub async fn cancel_event(
        state: State<'_, SharedState>,
        community_id: String,
        event_id: String,
    ) -> Result<Done<()>, CommandError> {
        IpcRequest::Community(CommunityRequest::Activities(ActivityRequest::EventCancel {
            community: community_id,
            event_id,
        }))
    }
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

host_command! {
    state;
    pub async fn set_event_rsvp(
        state: State<'_, SharedState>,
        community_id: String,
        event_id: String,
        status: String,
    ) -> Result<Done<()>, CommandError> {
        IpcRequest::Community(CommunityRequest::Activities(ActivityRequest::EventRsvp {
            community: community_id,
            event_id,
            status,
        }))
    }
}

host_command! {
    state;
    pub async fn list_event_attendees(
        state: State<'_, SharedState>,
        community_id: String,
        event_id: String,
    ) -> Result<Done<Vec<EventRsvp>>, CommandError> {
        IpcRequest::Community(CommunityRequest::Activities(
            ActivityRequest::EventAttendees {
                community: community_id,
                event_id,
            },
        ))
    }
}

host_command! {
    state;
    pub async fn get_events(
        state: State<'_, SharedState>,
        community_id: String,
    ) -> Result<Done<Vec<EventInfo>>, CommandError> {
        IpcRequest::Community(CommunityRequest::Activities(ActivityRequest::EventList {
            community: community_id,
        }))
    }
}

//! A community's event requests (plan E3.1 E4), answered by the Host's
//! governance adapter on both hosts.

use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_ipc::protocol::{CommunityRequest, IpcResponse};

use super::{adapter, state_error, DaemonContext};
use crate::daemon::DaemonState;

/// Answer one event request.
pub(crate) async fn handle(
    ctx: &DaemonContext,
    state: DaemonState,
    request: CommunityRequest,
) -> IpcResponse {
    let query = matches!(
        request,
        CommunityRequest::EventList { .. } | CommunityRequest::EventAttendees { .. }
    );
    if query && !state.can_query() {
        return state_error(state, "query");
    }
    if !query && !state.can_write() {
        return state_error(state, "write");
    }
    let community = match &request {
        CommunityRequest::EventList { community }
        | CommunityRequest::EventAttendees { community, .. }
        | CommunityRequest::EventCreate { community, .. }
        | CommunityRequest::EventEdit { community, .. }
        | CommunityRequest::EventCancel { community, .. }
        | CommunityRequest::EventDelete { community, .. }
        | CommunityRequest::EventRsvp { community, .. } => community.clone(),
        _ => return IpcResponse::error(400, "not an event request"),
    };
    let id = match ctx.resolve_community(&community) {
        Ok(m) => m.governance_key,
        Err(e) => return e,
    };
    let host = adapter(ctx);
    match request {
        CommunityRequest::EventList { .. } => answer(host.events(&id).await, "events"),
        CommunityRequest::EventAttendees { event_id, .. } => {
            answer(host.event_attendees(&id, &event_id), "event attendees")
        }
        CommunityRequest::EventCreate { event, .. } => {
            answer(host.create_event(&id, event).await, "event create")
        }
        CommunityRequest::EventEdit { event_id, edit, .. } => {
            answer(host.edit_event(&id, &event_id, edit).await, "event edit")
        }
        CommunityRequest::EventCancel { event_id, .. } => {
            answer(host.cancel_event(&id, &event_id).await, "event cancel")
        }
        CommunityRequest::EventDelete { event_id, .. } => {
            answer(host.delete_event(&id, &event_id).await, "event delete")
        }
        CommunityRequest::EventRsvp {
            event_id, status, ..
        } => answer(
            host.set_event_rsvp(&id, &event_id, &status).await,
            "event RSVP",
        ),
        _ => IpcResponse::error(400, "not an event request"),
    }
}

/// A result as the answer: 403 when we lack the permission.
fn answer<T: serde::Serialize>(
    result: Result<T, GovernanceRuntimeError>,
    what: &str,
) -> IpcResponse {
    match result {
        Ok(value) => IpcResponse::ok(&value),
        Err(e @ GovernanceRuntimeError::PermissionDenied) => {
            IpcResponse::error(403, format!("{what}: {e}"))
        }
        Err(e) => IpcResponse::error(500, format!("{what}: {e}")),
    }
}

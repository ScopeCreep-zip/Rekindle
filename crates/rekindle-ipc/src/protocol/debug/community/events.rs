//! The event and RSVP requests' `Debug` (plan E3.1): a title and
//! description show their length only.

use super::super::super::CommunityRequest;

impl CommunityRequest {
    /// The event and RSVP requests' `Debug`.
    pub(super) fn fmt_events(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EventList { community } => f
                .debug_struct("EventList")
                .field("community", community)
                .finish(),
            Self::EventAttendees {
                community,
                event_id,
            } => fmt_event(f, "EventAttendees", community, event_id),
            Self::EventCreate { community, event } => f
                .debug_struct("EventCreate")
                .field("community", community)
                .field("title_len", &event.title.len())
                .field("description_len", &event.description.len())
                .field("start_time", &event.start_time)
                .finish_non_exhaustive(),
            Self::EventEdit {
                community,
                event_id,
                edit,
            } => f
                .debug_struct("EventEdit")
                .field("community", community)
                .field("event_id", event_id)
                .field("title_len", &edit.title.as_ref().map(String::len))
                .field("start_time", &edit.start_time)
                .finish_non_exhaustive(),
            Self::EventCancel {
                community,
                event_id,
            } => fmt_event(f, "EventCancel", community, event_id),
            Self::EventDelete {
                community,
                event_id,
            } => fmt_event(f, "EventDelete", community, event_id),
            Self::EventRsvp {
                community,
                event_id,
                status,
            } => f
                .debug_struct("EventRsvp")
                .field("community", community)
                .field("event_id", event_id)
                .field("status", status)
                .finish(),
            other => unreachable!("{} is not an event request", other.name()),
        }
    }
}

/// An event request that names only the community and the event.
fn fmt_event(
    f: &mut std::fmt::Formatter<'_>,
    name: &str,
    community: &str,
    event_id: &str,
) -> std::fmt::Result {
    f.debug_struct(name)
        .field("community", &community)
        .field("event_id", &event_id)
        .finish()
}

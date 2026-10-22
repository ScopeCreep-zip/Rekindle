//! A community's event requests (plan E3.1 E4): the community key, the
//! event id, the title and description bounds of architecture §21, the
//! channel id, and the three RSVP answers.

use rekindle_ipc::protocol::ActivityRequest;
use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::event::{is_event_id, MAX_EVENT_DESCRIPTION_CHARS, MAX_EVENT_NAME_CHARS};

use super::{community, name, opt, Checked};

/// An event request's fields.
pub(super) fn check(request: &ActivityRequest) -> Checked {
    match request {
        ActivityRequest::EventList { community: key } => community(key),
        ActivityRequest::EventAttendees {
            community: key,
            event_id,
        }
        | ActivityRequest::EventCancel {
            community: key,
            event_id,
        }
        | ActivityRequest::EventDelete {
            community: key,
            event_id,
        } => {
            community(key)?;
            event(event_id)
        }
        ActivityRequest::EventCreate {
            community: key,
            event: new,
        } => {
            community(key)?;
            title(&new.title)?;
            description(&new.description)?;
            opt(new.channel_id.as_deref(), channel)
        }
        ActivityRequest::EventEdit {
            community: key,
            event_id,
            edit,
        } => {
            community(key)?;
            event(event_id)?;
            opt(edit.title.as_deref(), title)?;
            opt(edit.description.as_deref(), description)?;
            opt(edit.channel_id.as_deref(), channel)
        }
        ActivityRequest::EventRsvp {
            community: key,
            event_id,
            status,
        } => {
            community(key)?;
            event(event_id)?;
            if matches!(
                status.as_str(),
                "going" | "interested" | "maybe" | "declined"
            ) {
                Ok(())
            } else {
                Err(IpcResponse::error(
                    400,
                    "RSVP must be going, interested, maybe or declined",
                ))
            }
        }
        _ => Ok(()),
    }
}

fn event(id: &str) -> Checked {
    if is_event_id(id) {
        Ok(())
    } else {
        Err(IpcResponse::error(
            400,
            "event id must be evt_ and 32 hex digits",
        ))
    }
}

fn title(s: &str) -> Checked {
    name(s, "event title")?;
    chars(s, MAX_EVENT_NAME_CHARS, "event title")
}

fn description(s: &str) -> Checked {
    chars(s, MAX_EVENT_DESCRIPTION_CHARS, "event description")
}

fn chars(s: &str, max: usize, label: &str) -> Checked {
    if s.chars().count() > max {
        Err(IpcResponse::error(
            400,
            format!("{label} exceeds {max} characters"),
        ))
    } else {
        Ok(())
    }
}

fn channel(id: &str) -> Checked {
    if id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(IpcResponse::error(400, "channel id must be 32 hex digits"))
    }
}

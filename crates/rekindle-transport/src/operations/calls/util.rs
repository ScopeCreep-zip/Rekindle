//! Small pure helpers shared across the call runtime split: id
//! generation, error classification, and kind/status (de)serialization
//! for the persisted-call-state string columns.

use rekindle_calls::{CallKind, CallStatus};

pub(super) fn generate_call_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// W16.5b — classify a transport error from the `app_call` CallInvite
/// dispatch into the four `LocalUnreachable` reasons that drive the
/// `CallUnreachable` notification.
pub(super) fn classify_call_invite_error(e: &crate::error::TransportError) -> &'static str {
    use crate::error::TransportError;
    match e {
        TransportError::Timeout { .. } => "timeout",
        TransportError::NoRoute { .. } => "no_route",
        TransportError::SendFailed { reason, .. } => {
            // Inspect the underlying Veilid error string. Veilid maps
            // InvalidTarget / NoConnection through here.
            if reason.contains("InvalidTarget") || reason.contains("NoConnection") {
                "no_route"
            } else if reason.to_ascii_lowercase().contains("service") {
                "service_unavailable"
            } else {
                "send_failed"
            }
        }
        _ => "send_failed",
    }
}

/// Wire-string for [`CallKind`] — one spelling, owned by
/// `rekindle-calls` alongside the state machine that defines the kind.
pub(super) use rekindle_calls::state_machine::kind_str;

pub(super) fn status_str(s: CallStatus) -> &'static str {
    match s {
        CallStatus::Outgoing => "outgoing",
        CallStatus::Incoming => "incoming",
        CallStatus::Connecting => "connecting",
        CallStatus::Active => "active",
        CallStatus::Missed => "missed",
    }
}

/// Inverse of [`kind_str`]. Returns `None` for unrecognized strings.
pub(super) fn parse_kind(s: &str) -> Option<CallKind> {
    match s {
        "audio" => Some(CallKind::Audio),
        "video" => Some(CallKind::Video),
        _ => None,
    }
}

/// Inverse of [`status_str`]. Returns `None` for unrecognized strings.
pub(super) fn parse_status(s: &str) -> Option<CallStatus> {
    match s {
        "outgoing" => Some(CallStatus::Outgoing),
        "incoming" => Some(CallStatus::Incoming),
        "connecting" => Some(CallStatus::Connecting),
        "active" => Some(CallStatus::Active),
        "missed" => Some(CallStatus::Missed),
        _ => None,
    }
}

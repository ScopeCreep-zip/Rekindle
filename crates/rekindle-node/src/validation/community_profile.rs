//! Bounds on proposed community profile values. Clear and unchanged carry no value.

use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::dto::community_profile::{CommunityProfilePatch, ProfileField};
use rekindle_types::key_format::{self, MAX_STATUS_LEN};
use rekindle_types::presence::limits;

use super::Checked;

pub(super) fn display_name(edit: &ProfileField<String>) -> Checked {
    if let Some(name) = edit.value() {
        limits::peer_display_name(name.trim())
            .map_err(|error| IpcResponse::error(400, format!("display name: {error}")))?;
    }
    Ok(())
}

pub(super) fn patch(edit: &CommunityProfilePatch) -> Checked {
    for (label, field) in [
        ("bio", &edit.bio),
        ("pronouns", &edit.pronouns),
        ("custom status", &edit.custom_status),
    ] {
        if field.value().is_some_and(|value| value.trim().is_empty()) {
            return Err(IpcResponse::error(
                400,
                format!("{label}: use clear for empty text"),
            ));
        }
    }
    if edit.badges.value().is_some_and(Vec::is_empty) {
        return Err(IpcResponse::error(
            400,
            "badges: use clear for an empty list",
        ));
    }
    if let Some(status) = edit.custom_status.value() {
        key_format::text(status, MAX_STATUS_LEN)
            .map_err(|error| IpcResponse::error(400, format!("custom status: {error}")))?;
    }
    display_name(&edit.display_name)?;
    limits::validate_profile(
        edit.bio.value().map(String::as_str),
        edit.pronouns.value().map(String::as_str),
        edit.badges.value().map_or(&[], Vec::as_slice),
        edit.avatar_ref.value().map(String::as_str),
        edit.banner_ref.value().map(String::as_str),
    )
    .map_err(|error| IpcResponse::error(400, error))
}

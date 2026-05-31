//! Phase 19.i-REDO — thin facade.
//!
//! Mention parsing, permission gating, and notification escalation
//! live in `rekindle_channel::mentions`. This module constructs a
//! `ChannelAdapter` per call and delegates.
//!
//! `MentionMatches` is re-exported so existing callers in
//! channel_messages / threads / message_notifications continue to
//! compile against the same type.

use std::sync::Arc;

use tauri::Manager;

use crate::state::AppState;

pub use rekindle_channel::MentionMatches;

fn build_adapter(
    state: &Arc<AppState>,
) -> Option<crate::services::channel_adapter::ChannelAdapter> {
    let app_handle = state.app_handle.read().clone()?;
    let pool = app_handle.try_state::<crate::db::DbPool>()?.inner().clone();
    Some(crate::services::channel_adapter::ChannelAdapter::new(
        Arc::clone(state),
        app_handle,
        pool,
    ))
}

pub fn validate_sender_permissions(
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym_hex: &str,
    matches: &mut MentionMatches,
) {
    let Some(adapter) = build_adapter(state) else {
        return;
    };
    rekindle_channel::validate_sender_permissions(
        &adapter,
        community_id,
        sender_pseudonym_hex,
        matches,
    );
}

pub fn local_member_is_mentioned(
    state: &Arc<AppState>,
    community_id: &str,
    matches: &MentionMatches,
) -> bool {
    let Some(adapter) = build_adapter(state) else {
        return false;
    };
    rekindle_channel::local_member_is_mentioned(&adapter, community_id, matches)
}

pub fn matches_from_cleartext(
    state: &Arc<AppState>,
    community_id: &str,
    mentioned_pseudonyms: &[String],
    mentioned_roles: &[String],
    mention_everyone: bool,
    mention_here: bool,
) -> MentionMatches {
    let Some(adapter) = build_adapter(state) else {
        return MentionMatches {
            everyone: mention_everyone,
            here: mention_here,
            roles: mentioned_roles.iter().map(|r| r.to_lowercase()).collect(),
            members: Vec::new(),
        };
    };
    rekindle_channel::matches_from_cleartext(
        &adapter,
        community_id,
        mentioned_pseudonyms,
        mentioned_roles,
        mention_everyone,
        mention_here,
    )
}

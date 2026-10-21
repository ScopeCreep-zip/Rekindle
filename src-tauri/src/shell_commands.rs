//! The port ledger (plan E1, recipe step 6): every Tauri command in
//! `invoke.rs` is in exactly one table. `HOST_CALLS` reach the Host through
//! `HostApi` (`host_call::call`); `SHELL_LOCAL` is the shell's own (windows,
//! the tray, OS dialogs, the OS opener and launcher, deep-link reception,
//! autostart, the event subscription); `PENDING_PORT` names the step that
//! moves each remaining command onto the Host. F1 requires `PENDING_PORT`
//! empty. The test below keeps the tables true to `invoke.rs` and the
//! command table.

/// How a Host-call command reaches the Host. E4.1 adds `MediaUp`, the
/// client→Host encoded-media lane (`HostApi::media_up`), with its first
/// command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostCall {
    /// One `IpcRequest`, by its `name()`.
    Request(&'static str),
}

/// `(Tauri command, how it reaches the Host)`.
pub const HOST_CALLS: &[(&str, HostCall)] = &[
    ("create_identity", HostCall::Request("IdentityCreate")),
    ("login", HostCall::Request("Unlock")),
    ("get_identity", HostCall::Request("IdentityShow")),
    ("pqxdh_bundle_info", HostCall::Request("PrekeyStatus")),
    ("logout", HostCall::Request("Logout")),
    ("list_identities", HostCall::Request("IdentityList")),
    ("delete_identity", HostCall::Request("IdentityDelete")),
    ("audit_verify", HostCall::Request("AuditVerify")),
    ("audit_export", HostCall::Request("AuditExport")),
    ("lifecycle_current", HostCall::Request("Status")),
    ("intent_commit", HostCall::Request("IntentCommit")),
    ("intent_cancel", HostCall::Request("IntentCancel")),
    ("undo", HostCall::Request("Undo")),
    ("set_status", HostCall::Request("PresenceSet")),
    (
        "set_status_message",
        HostCall::Request("PresenceSetMessage"),
    ),
    ("set_nickname", HostCall::Request("ProfileSetNickname")),
    (
        "set_read_receipts_enabled",
        HostCall::Request("SetReadReceiptsEnabled"),
    ),
    ("set_avatar", HostCall::Request("ProfileSetAvatar")),
    ("get_avatar", HostCall::Request("AvatarGet")),
    ("get_game_status", HostCall::Request("PresenceGet")),
    ("open_external_url", HostCall::Request("LinkOpen")),
    // Friends and the 1:1 paths (plan E2.1).
    ("send_message", HostCall::Request("DmSend")),
    ("outbox_retry", HostCall::Request("OutboxRetry")),
    ("outbox_discard", HostCall::Request("OutboxDiscard")),
    ("send_typing", HostCall::Request("DmTyping")),
    ("get_message_history", HostCall::Request("DmHistory")),
    ("mark_read", HostCall::Request("DmMarkRead")),
    (
        "add_friend_from_code",
        HostCall::Request("FriendAddFromCode"),
    ),
    ("get_friend_code", HostCall::Request("FriendCode")),
    (
        "rotate_friend_token",
        HostCall::Request("FriendTokenRotate"),
    ),
    ("remove_friend", HostCall::Request("FriendRemove")),
    ("accept_request", HostCall::Request("FriendAccept")),
    ("reject_request", HostCall::Request("FriendReject")),
    ("cancel_request", HostCall::Request("FriendCancelRequest")),
    ("retry_handshake", HostCall::Request("FriendRetryHandshake")),
    ("get_friends", HostCall::Request("FriendList")),
    ("get_pending_requests", HostCall::Request("FriendRequests")),
    (
        "create_friend_group",
        HostCall::Request("FriendGroupCreate"),
    ),
    (
        "rename_friend_group",
        HostCall::Request("FriendGroupRename"),
    ),
    (
        "delete_friend_group",
        HostCall::Request("FriendGroupDelete"),
    ),
    ("move_friend_to_group", HostCall::Request("FriendGroupMove")),
    (
        "set_friend_nickname",
        HostCall::Request("FriendSetNickname"),
    ),
    ("generate_invite", HostCall::Request("FriendInviteCreate")),
    (
        "add_friend_from_invite",
        HostCall::Request("FriendAddFromInvite"),
    ),
    ("cancel_invite", HostCall::Request("FriendInviteCancel")),
    (
        "get_outgoing_invites",
        HostCall::Request("FriendInviteList"),
    ),
    ("block_user", HostCall::Request("FriendBlock")),
    ("unblock_user", HostCall::Request("FriendUnblock")),
    ("get_blocked_users", HostCall::Request("FriendBlockedList")),
    (
        "reset_signal_session",
        HostCall::Request("SessionResetStart"),
    ),
    // Safety numbers and verification (plan E2.3).
    (
        "get_safety_number",
        HostCall::Request("IdentitySafetyNumber"),
    ),
    (
        "mark_identity_verified",
        HostCall::Request("IdentityMarkVerified"),
    ),
    (
        "clear_identity_verification",
        HostCall::Request("IdentityClearVerification"),
    ),
];

/// The shell's own commands.
pub const SHELL_LOCAL: &[&str] = &[
    "subscribe_events",
    "show_buddy_list",
    "open_chat_window",
    "open_community_window",
    "open_settings_window",
    "open_profile_window",
    "open_call_window",
    "get_game_name",
    "launch_game_to_server",
];

/// `(Tauri command, the step that ports it)`.
pub const PENDING_PORT: &[(&str, &str)] = &[
    // E2.6 — the SMPL group-DM path (E2.3 deleted its 1:1 half) is replaced
    // by the group DM requests.
    ("accept_dm_invite", "E2.6"),
    ("decline_dm_invite", "E2.6"),
    ("list_dms", "E2.6"),
    ("get_dm_messages", "E2.6"),
    ("send_dm_message", "E2.6"),
    // E3.1 — communities.
    ("add_game_server", "E3.1"),
    ("remove_game_server", "E3.1"),
    ("get_game_servers", "E3.1"),
    ("add_reaction", "E3.1"),
    ("remove_reaction", "E3.1"),
    ("admin_delete_channel_message", "E3.1"),
    ("bulk_delete_channel_messages", "E3.1"),
    ("delete_channel_message", "E3.1"),
    ("edit_channel_message", "E3.1"),
    ("forward_channel_message", "E3.1"),
    ("archive_thread", "E3.1"),
    ("unarchive_thread", "E3.1"),
    ("create_thread", "E3.1"),
    ("get_active_threads", "E3.1"),
    ("get_channel_threads", "E3.1"),
    ("get_thread_messages", "E3.1"),
    ("send_thread_message", "E3.1"),
    ("assign_role", "E3.1"),
    ("unassign_role", "E3.1"),
    ("self_assign_role", "E3.1"),
    ("self_unassign_role", "E3.1"),
    ("create_role", "E3.1"),
    ("edit_role", "E3.1"),
    ("delete_role", "E3.1"),
    ("get_roles", "E3.1"),
    ("ban_member", "E3.1"),
    ("unban_member", "E3.1"),
    ("get_ban_list", "E3.1"),
    ("timeout_member", "E3.1"),
    ("remove_timeout", "E3.1"),
    ("remove_community_member", "E3.1"),
    ("cancel_event", "E3.1"),
    ("create_event", "E3.1"),
    ("delete_event", "E3.1"),
    ("edit_event", "E3.1"),
    ("get_events", "E3.1"),
    ("list_event_attendees", "E3.1"),
    ("rsvp_event", "E3.1"),
    ("set_event_rsvp", "E3.1"),
    ("close_poll", "E3.1"),
    ("create_poll", "E3.1"),
    ("get_poll_results", "E3.1"),
    ("vote_poll", "E3.1"),
    ("create_category", "E3.1"),
    ("delete_category", "E3.1"),
    ("rename_category", "E3.1"),
    ("reorder_categories", "E3.1"),
    ("create_channel", "E3.1"),
    ("delete_channel", "E3.1"),
    ("rename_channel", "E3.1"),
    ("move_channel", "E3.1"),
    ("reorder_channels", "E3.1"),
    ("set_channel_topic", "E3.1"),
    ("set_slowmode", "E3.1"),
    ("set_channel_forum_tags", "E3.1"),
    ("set_channel_overwrite", "E3.1"),
    ("delete_channel_overwrite", "E3.1"),
    ("create_community", "E3.1"),
    ("join_community", "E3.1"),
    ("leave_community", "E3.1"),
    ("get_communities", "E3.1"),
    ("get_community_details", "E3.1"),
    ("get_community_members", "E3.1"),
    ("update_community_info", "E3.1"),
    ("update_community_presence", "E3.1"),
    ("update_community_profile", "E3.1"),
    ("get_community_avatar_data_url", "E3.1"),
    ("set_community_avatar", "E3.1"),
    ("set_community_banner", "E3.1"),
    ("get_community_policy", "E3.1"),
    ("set_community_policy", "E3.1"),
    ("create_community_invite", "E3.1"),
    ("list_community_invites", "E3.1"),
    ("revoke_community_invite", "E3.1"),
    ("delete_automod_rule", "E3.1"),
    ("list_automod_rules", "E3.1"),
    ("set_automod_rule", "E3.1"),
    ("delete_emoji", "E3.1"),
    ("upload_emoji", "E3.1"),
    ("upload_sticker", "E3.1"),
    ("list_expressions", "E3.1"),
    ("get_audit_log", "E3.1"),
    ("get_channel_messages", "E3.1"),
    ("get_older_channel_messages", "E3.1"),
    ("send_channel_message", "E3.1"),
    ("send_channel_typing", "E3.1"),
    ("get_channel_pins", "E3.1"),
    ("pin_message", "E3.1"),
    ("unpin_message", "E3.1"),
    ("pin_attachment", "E3.1"),
    ("mark_channel_read", "E3.1"),
    ("get_unread_counts", "E3.1"),
    ("set_active_channel", "E3.1"),
    ("get_onboarding_config", "E3.1"),
    ("set_onboarding_config", "E3.1"),
    ("submit_onboarding_answers", "E3.1"),
    ("mark_onboarding_complete", "E3.1"),
    ("get_welcome_screen", "E3.1"),
    ("set_welcome_screen", "E3.1"),
    ("get_presence_policy", "E3.1"),
    ("set_presence_policy", "E3.1"),
    ("rotate_mek", "E3.1"),
    ("get_pending_deep_link", "E3.1"),
    ("confirm_deep_link", "E3.1"),
    ("dismiss_deep_link", "E3.1"),
    // Search reads both histories: it moves with the last of them.
    ("search_messages", "E3.1"),
    // E3.2 — plate gates.
    ("expand_community_segment", "E3.2"),
    // E4.1 — voice, calls, video.
    ("accept_dm_call", "E4.1"),
    ("accept_group_call", "E4.1"),
    ("decline_dm_call", "E4.1"),
    ("decline_group_call", "E4.1"),
    ("dm_peer_video_decode_codecs", "E4.1"),
    ("end_dm_call", "E4.1"),
    ("end_group_call", "E4.1"),
    ("get_active_call", "E4.1"),
    ("get_missed_calls", "E4.1"),
    ("mute_caller_temp", "E4.1"),
    ("send_call_media_state", "E4.1"),
    ("send_call_reaction", "E4.1"),
    ("start_dm_call", "E4.1"),
    ("start_group_call", "E4.1"),
    ("get_stage_hand_raises", "E4.1"),
    ("join_voice_channel", "E4.1"),
    ("leave_voice", "E4.1"),
    ("list_audio_devices", "E4.1"),
    ("request_to_speak", "E4.1"),
    ("respond_to_speak_request", "E4.1"),
    ("server_deafen_member", "E4.1"),
    ("server_mute_member", "E4.1"),
    ("set_audio_devices", "E4.1"),
    ("set_deafen", "E4.1"),
    ("set_input_channels", "E4.1"),
    ("set_mute", "E4.1"),
    ("set_voice_mode", "E4.1"),
    ("play_soundboard", "E4.1"),
    ("upload_soundboard_sound", "E4.1"),
    ("register_dm_video_channel", "E4.1"),
    ("unregister_dm_video_channel", "E4.1"),
    ("send_dm_video_frame", "E4.1"),
    ("register_community_video_channel", "E4.1"),
    ("unregister_community_video_channel", "E4.1"),
    ("send_video_frame", "E4.1"),
    ("send_video_keyframe_request", "E4.1"),
    ("default_media_capabilities", "E4.1"),
    ("derive_video_stream_id", "E4.1"),
    ("force_native_keyframes", "E4.1"),
    ("list_native_video_devices", "E4.1"),
    ("native_video_active", "E4.1"),
    ("native_video_capture_available", "E4.1"),
    ("notify_video_topology_change", "E4.1"),
    ("register_native_preview_channel", "E4.1"),
    ("unregister_native_preview_channel", "E4.1"),
    ("report_local_video_capabilities", "E4.1"),
    ("report_media_capture_error", "E4.1"),
    ("report_video_decoder_status", "E4.1"),
    ("report_video_encoder_status", "E4.1"),
    ("report_video_render_facts", "E4.1"),
    ("set_video_device", "E4.1"),
    ("start_native_video", "E4.1"),
    ("stop_native_video", "E4.1"),
    // E5.1 — attachments, voice messages, link previews.
    ("upload_attachment", "E5.1"),
    ("download_attachment", "E5.1"),
    ("reveal_downloaded_attachment", "E5.1"),
    ("send_voice_message", "E5.1"),
    ("get_voice_message_audio", "E5.1"),
    ("fetch_link_preview", "E5.1"),
    ("get_link_previews_enabled", "E5.1"),
    ("set_link_previews_enabled", "E5.1"),
    // E5.2 — sync, relays, notification prefs, analytics, diagnostics.
    ("ensure_personal_sync_record", "E5.2"),
    ("read_paired_devices", "E5.2"),
    ("write_paired_devices", "E5.2"),
    ("read_sync_manifest", "E5.2"),
    ("write_sync_manifest", "E5.2"),
    ("read_sync_preferences", "E5.2"),
    ("write_sync_preferences", "E5.2"),
    ("read_sync_read_state", "E5.2"),
    ("write_sync_read_state", "E5.2"),
    ("run_background_sync", "E5.2"),
    ("list_received_relay_offers", "E5.2"),
    ("list_volunteered_relay_friends", "E5.2"),
    ("revoke_relay", "E5.2"),
    ("volunteer_relay", "E5.2"),
    ("list_push_relay_registrations", "E5.2"),
    ("register_with_push_relay", "E5.2"),
    ("unregister_with_push_relay", "E5.2"),
    ("get_community_default_notification_level", "E5.2"),
    ("set_community_default_notification_level", "E5.2"),
    ("set_channel_notification_level", "E5.2"),
    ("get_do_not_disturb", "E5.2"),
    ("set_do_not_disturb", "E5.2"),
    ("get_notification_sound", "E5.2"),
    ("set_notification_sound", "E5.2"),
    ("get_quiet_hours", "E5.2"),
    ("set_quiet_hours", "E5.2"),
    ("get_community_analytics", "E5.2"),
    ("debug_gossip_state", "E5.2"),
    ("vault_diagnostics", "E5.2"),
    // E5.3 — pairing.
    ("accept_pairing_code", "E5.3"),
    ("generate_pairing_qr_svg", "E5.3"),
    ("start_pairing_session", "E5.3"),
    // E5.7 — the transport handler owns attachment state.
    ("get_network_status", "E5.7"),
    // E6.8 — preferences.
    ("get_preferences", "E6.8"),
    ("set_preferences", "E6.8"),
    // E7.4 — the updater.
    ("check_for_updates", "E7.4"),
];

/// The `generate_handler!` parser `build.rs` and the capability test use.
#[cfg(test)]
#[path = "../build_support/invoke_commands.rs"]
mod invoke_commands;

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::invoke_commands;
    use super::*;

    /// Every registered command is in exactly one table, every table row is
    /// a registered command, and every Host-call request is one the command
    /// table classifies.
    #[test]
    fn every_command_is_in_exactly_one_table() {
        let registered = invoke_commands::parse(include_str!("invoke.rs")).unwrap();
        let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
        for name in HOST_CALLS
            .iter()
            .map(|(n, _)| *n)
            .chain(SHELL_LOCAL.iter().copied())
            .chain(PENDING_PORT.iter().map(|(n, _)| *n))
        {
            *seen.entry(name).or_default() += 1;
        }
        for command in &registered {
            assert_eq!(
                seen.remove(command.name()),
                Some(1),
                "{} must be in exactly one table",
                command.name()
            );
        }
        assert!(seen.is_empty(), "not registered in invoke.rs: {seen:?}");

        for (command, HostCall::Request(request)) in HOST_CALLS {
            assert!(
                rekindle_types::intent::kind_of(request).is_some(),
                "{command}: {request} is not in COMMAND_TABLE"
            );
            assert!(
                request.starts_with(|c: char| c.is_ascii_uppercase()),
                "{command}: {request} is not an IpcRequest name"
            );
        }
    }
}

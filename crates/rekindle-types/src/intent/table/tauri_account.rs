//! The Tauri commands for status and profile, games, relays, search, device sync, push, preferences and windows.

use crate::intent::IntentKind;

/// `(command or request name, kind)`.
pub(crate) const TAURI_ACCOUNT_ROWS: &[(&str, Option<IntentKind>)] = &[
    ("set_status", Some(IntentKind::SetStatus)),
    ("set_nickname", Some(IntentKind::SetNickname)),
    ("get_bio", None),
    ("get_friend_bio", None),
    ("set_bio", Some(IntentKind::SetBio)),
    ("set_avatar", Some(IntentKind::SetAvatar)),
    ("get_avatar", None),
    ("get_skin", None),
    // Each forwards its Host request (`shell_commands.rs`), with that row's kind.
    ("list_skins", None),
    ("validate_skin_draft", None),
    ("skin_draft_start", None),
    ("export_skin_draft", None),
    ("import_skin_region", None),
    ("export_skin_schema", None),
    // Reads: `HostApi::call` (`rekindle-node/src/host/api.rs:56` ->
    // `DaemonContext::call`, `daemon/dispatch/context.rs:261-267`) turns a
    // skin action that is a feature into its own request before the
    // clearance check, so that request is classified and gated as itself
    // (plan SK2-A4); the rest are window operations, and a selection is
    // view state.
    ("run_skin_action", None),
    ("set_skin_selection", None),
    ("report_skin_apply_failed", None),
    // The Downloads view (plan E7.2 G12): each the Host request it sends.
    ("transfer_list", None),
    ("transfer_start", Some(IntentKind::DownloadAttachment)),
    ("transfer_stop", Some(IntentKind::PauseDownload)),
    (
        "transfer_open",
        Some(IntentKind::RevealDownloadedAttachment),
    ),
    ("transfer_remove", Some(IntentKind::RemoveDownload)),
    ("import_skin", Some(IntentKind::ImportSkin)),
    ("remove_skin", Some(IntentKind::RemoveSkin)),
    ("select_skin", Some(IntentKind::SetPreferences)),
    ("keep_skin", Some(IntentKind::SetPreferences)),
    ("set_login_skin", Some(IntentKind::SetLoginSkin)),
    ("set_status_message", Some(IntentKind::SetStatusMessage)),
    ("get_game_status", None),
    ("get_game_totals", None),
    ("get_friend_game_totals", None),
    ("set_game_detection", Some(IntentKind::SetPreferences)),
    ("set_share_current_game", Some(IntentKind::ShareCurrentGame)),
    ("get_hidden_games", None),
    ("activity_list", None),
    ("set_keep_activity_log", Some(IntentKind::SetPreferences)),
    ("hide_game", Some(IntentKind::SetPreferences)),
    ("unhide_game", Some(IntentKind::ShareCurrentGame)),
    ("get_game_name", None),
    ("launch_game", Some(IntentKind::LaunchGameToServer)),
    ("get_installed_games", None),
    ("send_game_invite", Some(IntentKind::SendDmMessage)),
    ("get_favourite_servers", None),
    ("get_infoview", None),
    ("add_favourite_server", Some(IntentKind::SetPreferences)),
    ("remove_favourite_server", Some(IntentKind::SetPreferences)),
    // The Servers view (plan E7.2 G3): read, and forgetting a server we
    // visited, local like a favourite's removal.
    ("servers_view", None),
    ("recent_server_remove", Some(IntentKind::SetPreferences)),
    ("servers_refresh", Some(IntentKind::RefreshServers)),
    ("volunteer_relay", Some(IntentKind::VolunteerRelay)),
    ("revoke_relay", Some(IntentKind::RevokeRelay)),
    ("list_received_relay_offers", None),
    ("list_volunteered_relay_friends", None),
    ("send_dm_video_frame", None),
    ("register_dm_video_channel", None),
    ("unregister_dm_video_channel", None),
    ("search_messages", None),
    ("local_search", None),
    ("ensure_personal_sync_record", None),
    (
        "start_pairing_session",
        Some(IntentKind::StartPairingSession),
    ),
    (
        "generate_pairing_qr_svg",
        Some(IntentKind::GeneratePairingQrSvg),
    ),
    ("accept_pairing_code", Some(IntentKind::AcceptPairingCode)),
    ("read_sync_manifest", None),
    ("write_sync_manifest", None),
    ("read_sync_preferences", None),
    (
        "write_sync_preferences",
        Some(IntentKind::WriteSyncPreferences),
    ),
    ("read_paired_devices", None),
    ("write_paired_devices", None),
    (
        "register_with_push_relay",
        Some(IntentKind::RegisterWithPushRelay),
    ),
    (
        "unregister_with_push_relay",
        Some(IntentKind::UnregisterWithPushRelay),
    ),
    ("list_push_relay_registrations", None),
    ("get_preferences", None),
    ("set_preferences", Some(IntentKind::SetPreferences)),
    ("check_for_updates", None),
    ("show_buddy_list", None),
    // Quitting the app is the shell's `Shutdown`: Routine, with no review,
    // like the tray's Quit (r9-intents §7.2: leaving is abandoning a task).
    ("quit_app", Some(IntentKind::Shutdown)),
    ("open_external_url", Some(IntentKind::OpenExternalUrl)),
    ("open_chat_window", None),
    ("open_settings_window", None),
    ("open_skin_editor_window", None),
    // The Host already answered the pack's LaunchPopup (plan SK2-A5).
    ("open_skin_popup", None),
    ("open_community_window", None),
    ("open_profile_window", None),
    ("open_call_window", None),
    ("get_network_status", None),
    ("vault_diagnostics", None),
];

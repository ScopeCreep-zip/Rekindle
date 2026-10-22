//! The Tauri commands for status and profile, games, relays, search, device sync, push, preferences and windows.

use crate::intent::IntentKind;

/// `(command or request name, kind)`.
pub(crate) const TAURI_ACCOUNT_ROWS: &[(&str, Option<IntentKind>)] = &[
    ("set_status", Some(IntentKind::SetStatus)),
    ("set_nickname", Some(IntentKind::SetNickname)),
    ("set_avatar", Some(IntentKind::SetAvatar)),
    ("get_avatar", None),
    ("set_status_message", Some(IntentKind::SetStatusMessage)),
    ("get_game_status", None),
    ("get_game_name", None),
    (
        "launch_game_to_server",
        Some(IntentKind::LaunchGameToServer),
    ),
    ("volunteer_relay", Some(IntentKind::VolunteerRelay)),
    ("revoke_relay", Some(IntentKind::RevokeRelay)),
    ("list_received_relay_offers", None),
    ("list_volunteered_relay_friends", None),
    ("send_dm_video_frame", None),
    ("register_dm_video_channel", None),
    ("unregister_dm_video_channel", None),
    ("search_messages", None),
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
    ("open_external_url", Some(IntentKind::OpenExternalUrl)),
    ("open_chat_window", None),
    ("open_settings_window", None),
    ("open_community_window", None),
    ("open_profile_window", None),
    ("open_call_window", None),
    ("get_network_status", None),
    ("vault_diagnostics", None),
];

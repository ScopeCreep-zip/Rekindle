//! Tauri IPC command registration.
//!
//! `generate_handler!` takes one flat list, so each command domain owns a
//! `*_commands!` macro listing its entries, and [`handler`] folds every
//! domain into that one list through `fold_commands!`. A new command goes in
//! its domain's macro; a new domain is a new macro plus one name in
//! [`handler`]. This file stays the single registration point: `build.rs`
//! (the ACL manifest), `tests/capability_policy.rs` and `cargo xtask
//! check-intent-table` read the entries between each domain macro's
//! `$next!([$($done)*` and `] $($rest)*) };` lines
//! (`build_support/invoke_commands.rs`). A domain macro left out of
//! [`handler`] is an unused macro, which `deny(warnings)` rejects.

use crate::commands;

/// Appends the next domain's entries to `$done`, then hands the rest on;
/// with no domain left, expands to the one `generate_handler!` list.
macro_rules! fold_commands {
    ([$($done:tt)*]) => {
        tauri::generate_handler![$($done)*]
    };
    ([$($done:tt)*] $domain:ident $($rest:ident)*) => {
        $domain!(fold_commands [$($done)*] $($rest)*)
    };
}

macro_rules! auth_commands {
    ($next:ident [$($done:tt)*] $($rest:ident)*) => { $next!([$($done)*
        commands::auth::create_identity,
        commands::auth::login,
        commands::auth::get_identity,
        commands::auth::logout,
        commands::auth::list_identities,
        commands::auth::delete_identity,
        #[cfg(debug_assertions)]
        commands::auth::pqxdh_bundle_info,
        commands::auth::audit_verify,
        commands::auth::audit_export,
        commands::auth::lifecycle_current,
    ] $($rest)*) };
}

macro_rules! event_commands {
    ($next:ident [$($done:tt)*] $($rest:ident)*) => { $next!([$($done)*
        commands::event::subscribe_events,
    ] $($rest)*) };
}

macro_rules! intent_commands {
    ($next:ident [$($done:tt)*] $($rest:ident)*) => { $next!([$($done)*
        commands::intent::intent_commit,
        commands::intent::intent_cancel,
        commands::intent::undo,
    ] $($rest)*) };
}

macro_rules! chat_commands {
    ($next:ident [$($done:tt)*] $($rest:ident)*) => { $next!([$($done)*
        commands::chat::send_message,
        commands::chat::outbox_retry,
        commands::chat::outbox_discard,
        commands::chat::send_typing,
        commands::chat::get_message_history,
        commands::chat::mark_read,
    ] $($rest)*) };
}

macro_rules! group_dm_commands {
    ($next:ident [$($done:tt)*] $($rest:ident)*) => { $next!([$($done)*
        commands::group_dm::group_dm_create,
        commands::group_dm::group_dm_accept,
        commands::group_dm::group_dm_decline,
        commands::group_dm::group_dm_leave,
        commands::group_dm::group_dm_send,
        commands::group_dm::group_dm_typing,
        commands::group_dm::group_dm_list,
        commands::group_dm::group_dm_messages,
    ] $($rest)*) };
}

macro_rules! friends_commands {
    ($next:ident [$($done:tt)*] $($rest:ident)*) => { $next!([$($done)*
        commands::friends::add_friend_from_code,
        commands::friends::get_friend_code,
        commands::friends::rotate_friend_token,
        commands::friends::remove_friend,
        commands::friends::accept_request,
        commands::friends::get_friends,
        commands::friends::reject_request,
        commands::friends::get_pending_requests,
        commands::friends::create_friend_group,
        commands::friends::rename_friend_group,
        commands::friends::delete_friend_group,
        commands::friends::set_friend_nickname,
        commands::friends::move_friend_to_group,
        commands::friends::generate_invite,
        commands::friends::add_friend_from_invite,
        commands::friends::block_user,
        commands::friends::unblock_user,
        commands::friends::get_blocked_users,
        commands::friends::cancel_request,
        commands::friends::retry_handshake,
        commands::friends::cancel_invite,
        commands::friends::get_outgoing_invites,
        commands::friends::reset_signal_session,
        commands::friends::get_safety_number,
        commands::friends::mark_identity_verified,
        commands::friends::clear_identity_verification,
    ] $($rest)*) };
}

macro_rules! community_commands {
    ($next:ident [$($done:tt)*] $($rest:ident)*) => { $next!([$($done)*
        commands::community::create_community,
        commands::community::join_community,
        commands::community::create_channel,
        commands::community::create_category,
        commands::community::delete_category,
        commands::community::rename_category,
        commands::community::move_channel,
        commands::community::reorder_categories,
        commands::community::create_community_invite,
        commands::community::revoke_community_invite,
        commands::community::list_community_invites,
        commands::community::send_channel_message,
        commands::community::edit_channel_message,
        commands::community::delete_channel_message,
        commands::community::get_channel_messages,
        commands::community::get_communities,
        commands::community::get_community_details,
        commands::community::get_community_members,
        commands::community::remove_community_member,
        commands::community::get_roles,
        commands::community::create_role,
        commands::community::edit_role,
        commands::community::delete_role,
        commands::community::assign_role,
        commands::community::unassign_role,
        commands::community::self_assign_role,
        commands::community::self_unassign_role,
        commands::community::timeout_member,
        commands::community::remove_timeout,
        commands::community::set_channel_overwrite,
        commands::community::delete_channel_overwrite,
        commands::community::set_slowmode,
        commands::community::admin_delete_channel_message,
        commands::community::bulk_delete_channel_messages,
        commands::community::forward_channel_message,
        commands::community::upload_attachment,
        commands::community::download_attachment,
        commands::community::get_voice_message_audio,
        commands::community::reveal_downloaded_attachment,
        commands::community::pin_attachment,
        commands::community::send_voice_message,
        commands::community::expand_community_segment,
        commands::community::leave_community,
        commands::community::delete_channel,
        commands::community::rename_channel,
        commands::community::update_community_info,
        commands::community::ban_member,
        commands::community::unban_member,
        commands::community::get_ban_list,
        commands::community::rotate_mek,
        commands::community::set_channel_notification_level,
        commands::community::set_community_default_notification_level,
        commands::community::set_notification_sound,
        commands::community::get_notification_sound,
        commands::community::set_do_not_disturb,
        commands::community::get_do_not_disturb,
        commands::community::get_community_default_notification_level,
        commands::community::set_quiet_hours,
        commands::community::get_quiet_hours,
        commands::community::add_reaction,
        commands::community::remove_reaction,
        commands::community::pin_message,
        commands::community::unpin_message,
        commands::community::get_channel_pins,
        commands::community::get_audit_log,
        commands::community::get_community_analytics,
        commands::community::fetch_link_preview,
        commands::community::get_link_previews_enabled,
        commands::community::set_link_previews_enabled,
        commands::community::run_background_sync,
        commands::community::list_automod_rules,
        commands::community::set_automod_rule,
        commands::community::delete_automod_rule,
        commands::community::block_community_member,
        commands::community::unblock_community_member,
        commands::community::list_community_blocks,
        commands::community::get_onboarding_config,
        commands::community::set_onboarding_config,
        commands::community::get_welcome_screen,
        commands::community::set_welcome_screen,
        commands::community::submit_onboarding_answers,
        commands::community::mark_onboarding_complete,
        commands::community::get_community_policy,
        commands::community::set_community_policy,
        commands::community::set_community_avatar,
        commands::community::set_community_banner,
        commands::community::get_community_avatar_data_url,
        #[cfg(debug_assertions)]
        commands::community::debug_gossip_state,
        commands::community::send_channel_typing,
        commands::community::set_active_channel,
        commands::community::set_presence_policy,
        commands::community::get_presence_policy,
        commands::community::update_community_presence,
        commands::community::update_community_profile,
        commands::community::get_older_channel_messages,
        commands::community::set_channel_topic,
        commands::community::set_channel_forum_tags,
        commands::community::reorder_channels,
        commands::community::create_poll,
        commands::community::vote_poll,
        commands::community::close_poll,
        commands::community::get_poll_results,
        commands::community::upload_emoji,
        commands::community::upload_sticker,
        commands::community::upload_soundboard_sound,
        commands::community::play_soundboard,
        commands::community::derive_video_stream_id,
        commands::community::default_media_capabilities,
        commands::community::send_video_frame,
        commands::community::send_video_keyframe_request,
        commands::community::native_video_capture_available,
        commands::community::list_native_video_devices,
        commands::community::native_video_active,
        commands::community::start_native_video,
        commands::community::stop_native_video,
        commands::community::set_video_device,
        commands::community::force_native_keyframes,
        commands::community::notify_video_topology_change,
        commands::community::register_community_video_channel,
        commands::community::unregister_community_video_channel,
        commands::community::register_native_preview_channel,
        commands::community::unregister_native_preview_channel,
        commands::community::report_local_video_capabilities,
        commands::community::report_video_decoder_status,
        commands::community::report_video_render_facts,
        commands::community::report_media_capture_error,
        commands::community::report_video_encoder_status,
        commands::community::delete_emoji,
        commands::community::list_expressions,
        commands::community::create_thread,
        commands::community::get_active_threads,
        commands::community::get_channel_threads,
        commands::community::send_thread_message,
        commands::community::get_thread_messages,
        commands::community::archive_thread,
        commands::community::unarchive_thread,
        commands::community::create_event,
        commands::community::edit_event,
        commands::community::delete_event,
        commands::community::cancel_event,
        commands::community::rsvp_event,
        commands::community::set_event_rsvp,
        commands::community::list_event_attendees,
        commands::community::get_events,
        commands::community::add_game_server,
        commands::community::remove_game_server,
        commands::community::get_game_servers,
        commands::community::mark_channel_read,
        commands::community::get_unread_counts,
    ] $($rest)*) };
}

macro_rules! voice_commands {
    ($next:ident [$($done:tt)*] $($rest:ident)*) => { $next!([$($done)*
        commands::voice::join_voice_channel,
        commands::voice::leave_voice,
        commands::voice::request_to_speak,
        commands::voice::get_stage_hand_raises,
        commands::voice::respond_to_speak_request,
        commands::voice::set_mute,
        commands::voice::set_deafen,
        commands::voice::list_audio_devices,
        commands::voice::set_audio_devices,
        commands::voice::set_input_channels,
        commands::voice::set_voice_mode,
        commands::voice::server_mute_member,
        commands::voice::server_deafen_member,
    ] $($rest)*) };
}

macro_rules! calls_commands {
    ($next:ident [$($done:tt)*] $($rest:ident)*) => { $next!([$($done)*
        commands::calls::start_dm_call,
        commands::calls::accept_dm_call,
        commands::calls::decline_dm_call,
        commands::calls::end_dm_call,
        commands::calls::send_call_media_state,
        commands::calls::send_call_reaction,
        commands::calls::mute_caller_temp,
        commands::calls::start_group_call,
        commands::calls::accept_group_call,
        commands::calls::decline_group_call,
        commands::calls::end_group_call,
        commands::calls::get_active_call,
        commands::calls::get_missed_calls,
        commands::calls::dm_peer_video_decode_codecs,
    ] $($rest)*) };
}

macro_rules! status_commands {
    ($next:ident [$($done:tt)*] $($rest:ident)*) => { $next!([$($done)*
        commands::status::set_status,
        commands::status::set_nickname,
        commands::status::get_bio,
        commands::status::get_friend_bio,
        commands::status::set_bio,
        commands::status::set_avatar,
        commands::status::get_avatar,
        commands::status::set_status_message,
    ] $($rest)*) };
}

macro_rules! skin_commands {
    ($next:ident [$($done:tt)*] $($rest:ident)*) => { $next!([$($done)*
        commands::skin::get_skin,
        commands::skin::list_skins,
        commands::skin::import_skin,
        commands::skin::remove_skin,
        commands::skin::select_skin,
        commands::skin::keep_skin,
        commands::skin::validate_skin_draft,
        commands::skin::skin_draft_start,
        commands::skin::export_skin_draft,
        commands::skin::import_skin_region,
        commands::skin::export_skin_schema,
        commands::skin::set_login_skin,
    ] $($rest)*) };
}

macro_rules! game_commands {
    ($next:ident [$($done:tt)*] $($rest:ident)*) => { $next!([$($done)*
        commands::game::get_game_status,
        commands::game::get_game_totals,
        commands::game::get_friend_game_totals,
        commands::game::get_hidden_games,
        commands::game::hide_game,
        commands::game::unhide_game,
        commands::game::get_game_name,
        commands::game::launch_game_to_server,
        commands::game::send_game_invite,
        commands::game::get_favourite_servers,
        commands::game::add_favourite_server,
        commands::game::remove_favourite_server,
    ] $($rest)*) };
}

macro_rules! relay_commands {
    ($next:ident [$($done:tt)*] $($rest:ident)*) => { $next!([$($done)*
        commands::relay::volunteer_relay,
        commands::relay::revoke_relay,
        commands::relay::list_received_relay_offers,
        commands::relay::list_volunteered_relay_friends,
    ] $($rest)*) };
}

macro_rules! dm_commands {
    ($next:ident [$($done:tt)*] $($rest:ident)*) => { $next!([$($done)*
        commands::dm::send_dm_video_frame,
        commands::dm::register_dm_video_channel,
        commands::dm::unregister_dm_video_channel,
    ] $($rest)*) };
}

macro_rules! search_commands {
    ($next:ident [$($done:tt)*] $($rest:ident)*) => { $next!([$($done)*
        commands::search::search_messages,
    ] $($rest)*) };
}

macro_rules! sync_commands {
    ($next:ident [$($done:tt)*] $($rest:ident)*) => { $next!([$($done)*
        commands::sync::ensure_personal_sync_record,
        commands::sync::start_pairing_session,
        commands::sync::generate_pairing_qr_svg,
        commands::sync::accept_pairing_code,
        commands::sync::read_sync_manifest,
        commands::sync::write_sync_manifest,
        commands::sync::read_sync_preferences,
        commands::sync::write_sync_preferences,
        commands::sync::read_paired_devices,
        commands::sync::write_paired_devices,
    ] $($rest)*) };
}

macro_rules! push_relay_commands {
    ($next:ident [$($done:tt)*] $($rest:ident)*) => { $next!([$($done)*
        commands::push_relay::register_with_push_relay,
        commands::push_relay::unregister_with_push_relay,
        commands::push_relay::list_push_relay_registrations,
    ] $($rest)*) };
}

macro_rules! settings_commands {
    ($next:ident [$($done:tt)*] $($rest:ident)*) => { $next!([$($done)*
        commands::settings::get_preferences,
        commands::settings::set_preferences,
        commands::settings::set_read_receipts_enabled,
        commands::settings::set_share_game_totals,
        commands::settings::set_game_detection,
        commands::settings::set_share_current_game,
        commands::settings::check_for_updates,
    ] $($rest)*) };
}

macro_rules! window_commands {
    ($next:ident [$($done:tt)*] $($rest:ident)*) => { $next!([$($done)*
        commands::window::show_buddy_list,
        commands::window::quit_app,
        commands::window::open_external_url,
        commands::window::open_chat_window,
        commands::window::open_settings_window,
        commands::window::open_skin_editor_window,
        commands::window::open_community_window,
        commands::window::open_profile_window,
        commands::window::open_call_window,
        commands::window::get_network_status,
    ] $($rest)*) };
}

macro_rules! vault_commands {
    ($next:ident [$($done:tt)*] $($rest:ident)*) => { $next!([$($done)*
        #[cfg(debug_assertions)]
        commands::vault::vault_diagnostics,
    ] $($rest)*) };
}

/// Build the invoke handler with every registered frontend → Rust command.
pub fn handler() -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    fold_commands!(
        []
        auth_commands
        event_commands
        intent_commands
        chat_commands
        group_dm_commands
        friends_commands
        community_commands
        voice_commands
        calls_commands
        status_commands
        skin_commands
        game_commands
        relay_commands
        dm_commands
        search_commands
        sync_commands
        push_relay_commands
        settings_commands
        window_commands
        vault_commands
    )
}

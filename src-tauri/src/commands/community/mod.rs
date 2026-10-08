mod analytics;
mod audit;
pub(crate) mod automod;
mod background_sync;
mod channel_admin;
mod channels;
mod crud;
#[cfg(debug_assertions)]
mod diagnostics;
mod events;
pub(crate) mod expressions;
mod files;
mod game_servers;
pub(crate) mod helpers;
mod invites;
mod link_previews;
mod mek;
mod messaging;
mod moderation;
mod notifications;
mod onboarding;
pub(crate) mod policy;
mod polls;
mod presence;
mod profile_blobs;
mod reactions_pins;
mod roles;
mod segments;
mod threads;
pub(crate) mod types;
mod unread;
mod video;

pub use analytics::{
    __cmd__get_community_analytics, __tauri_command_name_get_community_analytics,
    get_community_analytics,
};
pub use audit::{__cmd__get_audit_log, __tauri_command_name_get_audit_log, get_audit_log};
pub use automod::{
    __cmd__delete_automod_rule, __cmd__list_automod_rules, __cmd__set_automod_rule,
    __tauri_command_name_delete_automod_rule, __tauri_command_name_list_automod_rules,
    __tauri_command_name_set_automod_rule, delete_automod_rule, list_automod_rules,
    set_automod_rule,
};
pub use background_sync::{
    __cmd__run_background_sync, __tauri_command_name_run_background_sync, run_background_sync,
};
pub use channel_admin::{
    __cmd__delete_channel, __cmd__rename_channel, __tauri_command_name_delete_channel,
    __tauri_command_name_rename_channel, delete_channel, rename_channel,
};
pub use channels::{
    __cmd__create_category, __cmd__create_channel, __cmd__delete_category, __cmd__move_channel,
    __cmd__rename_category, __cmd__reorder_categories, __cmd__reorder_channels,
    __cmd__set_channel_forum_tags, __cmd__set_channel_topic, __tauri_command_name_create_category,
    __tauri_command_name_create_channel, __tauri_command_name_delete_category,
    __tauri_command_name_move_channel, __tauri_command_name_rename_category,
    __tauri_command_name_reorder_categories, __tauri_command_name_reorder_channels,
    __tauri_command_name_set_channel_forum_tags, __tauri_command_name_set_channel_topic,
};
pub use channels::{
    create_category, create_channel, delete_category, move_channel, rename_category,
    reorder_categories, reorder_channels, set_channel_forum_tags, set_channel_topic,
};
pub use crud::{
    __cmd__create_community, __cmd__get_communities, __cmd__get_community_details,
    __cmd__join_community, __cmd__leave_community, __cmd__update_community_info,
    __tauri_command_name_create_community, __tauri_command_name_get_communities,
    __tauri_command_name_get_community_details, __tauri_command_name_join_community,
    __tauri_command_name_leave_community, __tauri_command_name_update_community_info,
};
pub use crud::{
    create_community, get_communities, get_community_details, join_community, leave_community,
    update_community_info,
};
#[cfg(debug_assertions)]
pub use diagnostics::{
    __cmd__debug_gossip_state, __tauri_command_name_debug_gossip_state, debug_gossip_state,
};
pub use events::{
    __cmd__cancel_event, __cmd__create_event, __cmd__delete_event, __cmd__edit_event,
    __cmd__get_events, __cmd__list_event_attendees, __cmd__rsvp_event, __cmd__set_event_rsvp,
    __tauri_command_name_cancel_event, __tauri_command_name_create_event,
    __tauri_command_name_delete_event, __tauri_command_name_edit_event,
    __tauri_command_name_get_events, __tauri_command_name_list_event_attendees,
    __tauri_command_name_rsvp_event, __tauri_command_name_set_event_rsvp,
};
pub use events::{
    cancel_event, create_event, delete_event, edit_event, get_events, list_event_attendees,
    rsvp_event, set_event_rsvp,
};
pub use expressions::{
    __cmd__delete_emoji, __cmd__list_expressions, __cmd__play_soundboard, __cmd__upload_emoji,
    __cmd__upload_soundboard_sound, __cmd__upload_sticker, __tauri_command_name_delete_emoji,
    __tauri_command_name_list_expressions, __tauri_command_name_play_soundboard,
    __tauri_command_name_upload_emoji, __tauri_command_name_upload_soundboard_sound,
    __tauri_command_name_upload_sticker, delete_emoji, list_expressions, play_soundboard,
    upload_emoji, upload_soundboard_sound, upload_sticker,
};
pub use files::{
    __cmd__download_attachment, __cmd__get_voice_message_audio, __cmd__pin_attachment,
    __cmd__reveal_downloaded_attachment, __cmd__send_voice_message, __cmd__upload_attachment,
    __tauri_command_name_download_attachment, __tauri_command_name_get_voice_message_audio,
    __tauri_command_name_pin_attachment, __tauri_command_name_reveal_downloaded_attachment,
    __tauri_command_name_send_voice_message, __tauri_command_name_upload_attachment,
    download_attachment, get_voice_message_audio, pin_attachment, reveal_downloaded_attachment,
    send_voice_message, upload_attachment,
};
pub use game_servers::{
    __cmd__add_game_server, __cmd__get_game_servers, __cmd__remove_game_server,
    __tauri_command_name_add_game_server, __tauri_command_name_get_game_servers,
    __tauri_command_name_remove_game_server,
};
pub use game_servers::{add_game_server, get_game_servers, remove_game_server};
pub use invites::{
    __cmd__create_community_invite, __cmd__list_community_invites, __cmd__revoke_community_invite,
    __tauri_command_name_create_community_invite, __tauri_command_name_list_community_invites,
    __tauri_command_name_revoke_community_invite,
};
pub use invites::{create_community_invite, list_community_invites, revoke_community_invite};
pub use link_previews::{
    __cmd__fetch_link_preview, __cmd__get_link_previews_enabled, __cmd__set_link_previews_enabled,
    __tauri_command_name_fetch_link_preview, __tauri_command_name_get_link_previews_enabled,
    __tauri_command_name_set_link_previews_enabled, fetch_link_preview, get_link_previews_enabled,
    set_link_previews_enabled,
};
pub use mek::{__cmd__rotate_mek, __tauri_command_name_rotate_mek, rotate_mek};
pub use messaging::{
    __cmd__delete_channel_message, __cmd__edit_channel_message, __cmd__forward_channel_message,
    __cmd__get_channel_messages, __cmd__get_older_channel_messages, __cmd__send_channel_message,
    __tauri_command_name_delete_channel_message, __tauri_command_name_edit_channel_message,
    __tauri_command_name_forward_channel_message, __tauri_command_name_get_channel_messages,
    __tauri_command_name_get_older_channel_messages, __tauri_command_name_send_channel_message,
};
pub use messaging::{
    delete_channel_message, edit_channel_message, forward_channel_message, get_channel_messages,
    get_older_channel_messages, send_channel_message,
};
pub use moderation::{
    __cmd__admin_delete_channel_message, __cmd__ban_member, __cmd__bulk_delete_channel_messages,
    __cmd__delete_channel_overwrite, __cmd__get_ban_list, __cmd__remove_community_member,
    __cmd__remove_timeout, __cmd__set_channel_overwrite, __cmd__set_slowmode,
    __cmd__timeout_member, __cmd__unban_member, __tauri_command_name_admin_delete_channel_message,
    __tauri_command_name_ban_member, __tauri_command_name_bulk_delete_channel_messages,
    __tauri_command_name_delete_channel_overwrite, __tauri_command_name_get_ban_list,
    __tauri_command_name_remove_community_member, __tauri_command_name_remove_timeout,
    __tauri_command_name_set_channel_overwrite, __tauri_command_name_set_slowmode,
    __tauri_command_name_timeout_member, __tauri_command_name_unban_member,
};
pub use moderation::{
    admin_delete_channel_message, ban_member, bulk_delete_channel_messages,
    delete_channel_overwrite, get_ban_list, remove_community_member, remove_timeout,
    set_channel_overwrite, set_slowmode, timeout_member, unban_member,
};
pub use notifications::{
    __cmd__get_community_default_notification_level, __cmd__get_do_not_disturb,
    __cmd__get_notification_sound, __cmd__get_quiet_hours, __cmd__set_channel_notification_level,
    __cmd__set_community_default_notification_level, __cmd__set_do_not_disturb,
    __cmd__set_notification_sound, __cmd__set_quiet_hours,
    __tauri_command_name_get_community_default_notification_level,
    __tauri_command_name_get_do_not_disturb, __tauri_command_name_get_notification_sound,
    __tauri_command_name_get_quiet_hours, __tauri_command_name_set_channel_notification_level,
    __tauri_command_name_set_community_default_notification_level,
    __tauri_command_name_set_do_not_disturb, __tauri_command_name_set_notification_sound,
    __tauri_command_name_set_quiet_hours, get_community_default_notification_level,
    get_do_not_disturb, get_notification_sound, get_quiet_hours, set_channel_notification_level,
    set_community_default_notification_level, set_do_not_disturb, set_notification_sound,
    set_quiet_hours,
};
pub use onboarding::{
    __cmd__get_onboarding_config, __cmd__get_welcome_screen, __cmd__mark_onboarding_complete,
    __cmd__set_onboarding_config, __cmd__set_welcome_screen, __cmd__submit_onboarding_answers,
    __tauri_command_name_get_onboarding_config, __tauri_command_name_get_welcome_screen,
    __tauri_command_name_mark_onboarding_complete, __tauri_command_name_set_onboarding_config,
    __tauri_command_name_set_welcome_screen, __tauri_command_name_submit_onboarding_answers,
};
pub use onboarding::{
    get_onboarding_config, get_welcome_screen, mark_onboarding_complete, set_onboarding_config,
    set_welcome_screen, submit_onboarding_answers,
};
pub use policy::{
    __cmd__get_community_policy, __cmd__set_community_policy,
    __tauri_command_name_get_community_policy, __tauri_command_name_set_community_policy,
    get_community_policy, set_community_policy,
};
pub use polls::{
    __cmd__close_poll, __cmd__create_poll, __cmd__get_poll_results, __cmd__vote_poll,
    __tauri_command_name_close_poll, __tauri_command_name_create_poll,
    __tauri_command_name_get_poll_results, __tauri_command_name_vote_poll, close_poll, create_poll,
    get_poll_results, vote_poll,
};
pub use presence::{
    __cmd__get_community_members, __cmd__get_presence_policy, __cmd__send_channel_typing,
    __cmd__set_active_channel, __cmd__set_presence_policy, __cmd__update_community_presence,
    __cmd__update_community_profile, __tauri_command_name_get_community_members,
    __tauri_command_name_get_presence_policy, __tauri_command_name_send_channel_typing,
    __tauri_command_name_set_active_channel, __tauri_command_name_set_presence_policy,
    __tauri_command_name_update_community_presence, __tauri_command_name_update_community_profile,
};
pub use presence::{
    get_community_members, get_presence_policy, send_channel_typing, set_active_channel,
    set_presence_policy, update_community_presence, update_community_profile,
};
pub use profile_blobs::{
    __cmd__get_community_avatar_data_url, __cmd__set_community_avatar, __cmd__set_community_banner,
    __tauri_command_name_get_community_avatar_data_url, __tauri_command_name_set_community_avatar,
    __tauri_command_name_set_community_banner, get_community_avatar_data_url, set_community_avatar,
    set_community_banner,
};
pub use reactions_pins::{
    __cmd__add_reaction, __cmd__get_channel_pins, __cmd__pin_message, __cmd__remove_reaction,
    __cmd__unpin_message, __tauri_command_name_add_reaction, __tauri_command_name_get_channel_pins,
    __tauri_command_name_pin_message, __tauri_command_name_remove_reaction,
    __tauri_command_name_unpin_message,
};
pub use reactions_pins::{
    add_reaction, get_channel_pins, pin_message, remove_reaction, unpin_message,
};
pub use roles::{
    __cmd__assign_role, __cmd__create_role, __cmd__delete_role, __cmd__edit_role, __cmd__get_roles,
    __cmd__self_assign_role, __cmd__self_unassign_role, __cmd__unassign_role,
    __tauri_command_name_assign_role, __tauri_command_name_create_role,
    __tauri_command_name_delete_role, __tauri_command_name_edit_role,
    __tauri_command_name_get_roles, __tauri_command_name_self_assign_role,
    __tauri_command_name_self_unassign_role, __tauri_command_name_unassign_role,
};
pub use roles::{
    assign_role, create_role, delete_role, edit_role, get_roles, self_assign_role,
    self_unassign_role, unassign_role,
};
pub use segments::{
    __cmd__expand_community_segment, __tauri_command_name_expand_community_segment,
    expand_community_segment,
};
pub use threads::{
    __cmd__archive_thread, __cmd__create_thread, __cmd__get_active_threads,
    __cmd__get_channel_threads, __cmd__get_thread_messages, __cmd__send_thread_message,
    __cmd__unarchive_thread, __tauri_command_name_archive_thread,
    __tauri_command_name_create_thread, __tauri_command_name_get_active_threads,
    __tauri_command_name_get_channel_threads, __tauri_command_name_get_thread_messages,
    __tauri_command_name_send_thread_message, __tauri_command_name_unarchive_thread,
};
pub use threads::{
    archive_thread, create_thread, get_active_threads, get_channel_threads, get_thread_messages,
    send_thread_message, unarchive_thread,
};
pub use types::*;
pub use unread::{
    __cmd__get_unread_counts, __cmd__mark_channel_read, __tauri_command_name_get_unread_counts,
    __tauri_command_name_mark_channel_read,
};
pub use unread::{get_unread_counts, mark_channel_read};
pub use video::{
    __cmd__default_media_capabilities, __cmd__derive_video_stream_id,
    __cmd__force_native_keyframes, __cmd__list_native_video_devices, __cmd__native_video_active,
    __cmd__native_video_capture_available, __cmd__notify_video_topology_change,
    __cmd__register_community_video_channel, __cmd__register_native_preview_channel,
    __cmd__report_local_video_capabilities, __cmd__report_media_capture_error,
    __cmd__report_video_decoder_status, __cmd__report_video_encoder_status,
    __cmd__send_video_frame, __cmd__send_video_keyframe_request, __cmd__start_native_video,
    __cmd__stop_native_video, __cmd__unregister_community_video_channel,
    __cmd__unregister_native_preview_channel, __tauri_command_name_default_media_capabilities,
    __tauri_command_name_derive_video_stream_id, __tauri_command_name_force_native_keyframes,
    __tauri_command_name_list_native_video_devices, __tauri_command_name_native_video_active,
    __tauri_command_name_native_video_capture_available,
    __tauri_command_name_notify_video_topology_change,
    __tauri_command_name_register_community_video_channel,
    __tauri_command_name_register_native_preview_channel,
    __tauri_command_name_report_local_video_capabilities,
    __tauri_command_name_report_media_capture_error,
    __tauri_command_name_report_video_decoder_status,
    __tauri_command_name_report_video_encoder_status, __tauri_command_name_send_video_frame,
    __tauri_command_name_send_video_keyframe_request, __tauri_command_name_start_native_video,
    __tauri_command_name_stop_native_video,
    __tauri_command_name_unregister_community_video_channel,
    __tauri_command_name_unregister_native_preview_channel, default_media_capabilities,
    derive_video_stream_id, force_native_keyframes, list_native_video_devices, native_video_active,
    native_video_capture_available, notify_video_topology_change, register_community_video_channel,
    register_native_preview_channel, report_local_video_capabilities, report_media_capture_error,
    report_video_decoder_status, report_video_encoder_status, send_video_frame,
    send_video_keyframe_request, start_native_video, stop_native_video,
    unregister_community_video_channel, unregister_native_preview_channel,
};

pub(crate) use helpers::require_permission;

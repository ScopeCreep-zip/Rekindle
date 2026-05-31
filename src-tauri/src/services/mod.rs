// Phase 14.p — `calls`, `voice`, `group_calls` modules deleted.
// All voice / call protocol logic lives in `rekindle-voice` /
// `rekindle-calls` crates; the adapters below bridge AppState ↔ crate.
pub mod community;
pub mod community_channel_admin_runtime; // Phase 23.C — channel delete/rename orchestration lifted from commands/community/channel_admin.rs.
pub mod community_channel_runtime; // Phase 23.C — channel creation runtime orchestration lifted from commands/community/channels.rs.
pub mod community_audit_runtime; // Phase 23.C — audit-log read + ban-list runtime orchestration lifted from commands/community/{audit,moderation}.rs.
pub mod community_automod_runtime; // Phase 23.C — automod rule list/set/delete orchestration lifted from commands/community/automod.rs.
pub mod community_event_runtime; // Phase 23.C — event creation orchestration lifted from commands/community/events.rs.
pub mod community_game_servers_runtime; // Phase 23.C — game-server add/list orchestration lifted from commands/community/game_servers.rs.
pub mod community_files_runtime; // Phase 23.C — Lost Cargo file-handler orchestration lifted from commands/community/files.rs.
pub mod community_invite_runtime; // Phase 23.C — invite-handler runtime orchestration lifted from commands/community/invites.rs.
pub mod community_link_previews_runtime; // Phase 23.C — link-preview settings SQLite ops lifted from commands/community/link_previews.rs.
pub mod community_mek_local_rotate; // Phase 23.C — `rotate_mek_local` lifted from legacy/control.rs.
pub mod community_notifications_runtime; // Phase 23.C — quiet-hours wrappers lifted from commands/community/notifications.rs.
pub mod event_resume_runtime; // Phase 23.C — event_resume orchestration lifted from commands/event.rs.
pub mod community_onboarding_mappers; // Phase 23.C — pure protocol↔governance onboarding DTO converters lifted from legacy/onboarding.rs.
pub mod community_onboarding_runtime; // Phase 23.C — onboarding-handler runtime orchestration lifted from commands/community/onboarding.rs.
pub mod community_pins_runtime; // Phase 23.C — pin/unpin/get_channel_pins orchestration lifted from commands/community/reactions_pins.rs.
pub mod community_onboarding_validation; // Phase 23.C — pure onboarding validator lifted from commands/community/onboarding.rs.
pub mod community_presence_runtime; // Phase 23.C — presence-handler runtime orchestration lifted from commands/community/presence.rs.
pub mod community_profile_blobs_runtime; // Phase 23.C — community avatar/banner blob compression + content-addressed cache lifted from commands/community/profile_blobs.rs.
pub mod community_profile_validation; // Phase 23.C — pure profile validators lifted from commands/community/presence.rs.
pub mod community_lifecycle_runtime; // Phase 23.C — community-leave teardown orchestration lifted from commands/community/crud.rs.
pub mod community_moderation_runtime; // Phase 23.C — message-deletion runtime orchestration lifted from commands/community/moderation.rs.
pub mod community_moderation_bulk; // Phase 23.D.4 — bulk channel-message delete extracted from community_moderation_runtime.
pub mod community_registry_slot; // Phase 23.C — `clear_registry_presence_slot` helper lifted from legacy/messages.rs.
pub mod community_role_runtime; // Phase 23.C — role-mutation runtime orchestration lifted from commands/community/roles.rs.
pub mod community_role_handlers_runtime; // Phase 23.C — role-command Tauri handler wrappers (parse + permission + delegate) lifted from commands/community/roles.rs.
pub mod community_diagnostics_runtime; // Phase 23.C — debug_gossip_state body lifted from commands/community/diagnostics.rs.
pub mod community_unread_runtime; // Phase 23.C — unread mark/get bodies lifted from commands/community/unread.rs.
pub mod community_policy_runtime; // Phase 23.C — community policy get/set bodies lifted from commands/community/policy.rs.
pub mod community_views_runtime; // Phase 23.C — community list/detail DTO mappers lifted from commands/community/crud.rs.
pub mod community_video_runtime; // Phase 23.C — video-handler runtime orchestration lifted from commands/community/video.rs.
pub mod auth_cores; // Phase 23.C — create_identity_core + login_core helpers lifted from commands/auth.rs.
pub mod auth_runtime; // Phase 23.C — logout + delete_identity orchestration lifted from commands/auth.rs.
pub mod chat_runtime; // Phase 23.C — chat-handler runtime orchestration lifted from commands/chat.rs.
pub mod friend_runtime; // Phase 23.C — friend-handler runtime orchestration lifted from commands/friends.rs.
pub mod messaging_runtime; // Phase 23.C — channel-messaging runtime orchestration lifted from commands/community/messaging.rs.
pub mod friendship;
pub mod friendship_deps;
pub mod cross_device_sync;
pub mod dht_publish_service;
pub mod dm;
pub mod dm_adapter; // Phase 13 — DmDeps + DmMekCache impls.
pub mod dm_runtime; // Phase 23.C — DM video-frame command orchestration lifted from commands/dm.rs.
pub mod files_adapter; // Phase 15 — FilesDeps impl + Tier-9 facades.
pub mod mek_adapter; // Phase 17 — MekDistributeDeps impl + ChannelMekCache wrapper + Stronghold persist.
pub mod governance_adapter; // Phase 18 — GovernanceRuntimeDeps impl for community lifecycle ops.
pub mod channel_adapter; // Phase 19.h-REDO — ChannelMessagingDeps impl for channel messaging ops.
pub mod gossip_adapter; // Phase 20.e-REDO — GossipDeps impl for mesh broadcast.
pub mod presence_adapter; // Phase 21.e-REDO — FriendPresenceDeps impl for friend presence.
pub mod sync_adapter; // Phase 22.f-REDO — SyncDeps impl for pending-message retry.
pub mod status_runtime; // Phase 23.C — status-handler runtime orchestration lifted from commands/status.rs.
pub mod sync_runtime; // Phase 23.C — sync-handler runtime orchestration lifted from commands/sync.rs.
pub mod video_adapter; // Phase 16 — VideoDeps impl + send/receive facades.
pub mod voice_adapter; // Phase 14 — VoiceSessionDeps impl + voice free-fn facades.
pub mod voice_runtime; // Phase 23.C — voice auxiliary helpers (audio device list/prefs + stage audience gate).
pub mod window_runtime; // Phase 23.C — window helpers (get_network_status body).
pub mod call_runtime; // Phase 23.C — mid-call signaling orchestration lifted from commands/calls.rs.
pub mod calls_adapter; // Phase 14 — CallSignalingDeps impl + calls free-fn facades.
pub mod voice_signaling_adapter; // Phase 14.k — VoiceSignalingDeps impl + signaling facade.
pub mod game_publisher;
pub mod game_service;
pub mod idle_service;
pub mod login_runtime;
mod login_spawn;
pub mod message_service;
pub mod presence_service;
pub mod push_relay;
pub mod relay;
pub mod search;
pub mod sync_service;
pub mod sync_communities;
pub mod veilid;

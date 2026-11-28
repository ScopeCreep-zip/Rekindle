// Phase 14.p — `calls`, `voice`, `group_calls` modules deleted.
// All voice / call protocol logic lives in `rekindle-voice` /
// `rekindle-calls` crates; the adapters below bridge AppState ↔ crate.
pub mod call_runtime; // Phase 23.C — mid-call signaling orchestration lifted from commands/calls.rs.
pub mod calls_adapter; // Phase 14 — CallSignalingDeps impl + calls free-fn facades.
pub mod community;
#[cfg(debug_assertions)]
pub mod community_diagnostics_runtime; // Phase 23.C — debug_gossip_state body lifted from commands/community/diagnostics.rs.
pub mod community_files_runtime; // Phase 23.C — Lost Cargo file-handler orchestration lifted from commands/community/files.rs.
pub mod community_link_previews_runtime; // Phase 23.C — link-preview settings SQLite ops lifted from commands/community/link_previews.rs.
pub mod community_mek_local_rotate; // Phase 23.C — `rotate_mek_local` lifted from legacy/control.rs.
pub mod community_notifications_runtime; // Phase 23.C — quiet-hours wrappers lifted from commands/community/notifications.rs.
pub mod community_video_runtime; // Phase 23.C — video-handler runtime orchestration lifted from commands/community/video.rs.
pub mod cross_device_sync;
pub mod dm;
pub mod dm_runtime; // Phase 23.C — DM video-frame command orchestration lifted from commands/dm.rs.
pub mod mek_adapter; // Phase 17 — MekDistributeDeps impl + ChannelMekCache wrapper + Stronghold persist.
pub mod message_service;
pub mod native_video; // Linux-native GStreamer camera capture facade (plan rosy-bouncing-kahn Phase 3).
pub mod preferences; // Plan D1 — the unlocked identity's preferences.
pub mod presence_adapter; // Phase 21.e-REDO — FriendPresenceDeps impl for friend presence.
pub mod push_relay;
pub mod relay;
pub mod shell_session; // Plan E1.1 — the GUI's ShellHooks: its unported unlock stages.
pub mod sync_communities;
pub mod sync_runtime; // Phase 23.C — sync-handler runtime orchestration lifted from commands/sync.rs.
pub mod veilid;
pub mod video_adapter; // Phase 16 — VideoDeps impl + send/receive facades.
pub mod video_allocation;
pub mod voice_runtime; // Phase 23.C — voice auxiliary helpers (audio device list/prefs + stage audience gate).
pub mod window_runtime; // Phase 23.C — window helpers (get_network_status body).

// Phase 14.p — `calls`, `voice`, `group_calls` modules deleted.
// All voice / call protocol logic lives in `rekindle-voice` /
// `rekindle-calls` crates; the adapters below bridge AppState ↔ crate.
#[cfg(debug_assertions)]
pub mod call_runtime; // Phase 23.C — mid-call signaling orchestration lifted from commands/calls.rs.
pub mod calls_adapter; // Phase 14 — CallSignalingDeps impl + calls free-fn facades.
pub mod channel_adapter; // Phase 19.h-REDO — ChannelMessagingDeps impl for channel messaging ops.
pub mod community;
pub mod community_audit_runtime; // Phase 23.C — audit-log read + ban-list runtime orchestration lifted from commands/community/{audit,moderation}.rs.
pub mod community_automod_runtime; // Phase 23.C — automod rule list/set/delete orchestration lifted from commands/community/automod.rs.
#[cfg(debug_assertions)]
pub mod community_diagnostics_runtime; // Phase 23.C — debug_gossip_state body lifted from commands/community/diagnostics.rs.
pub mod community_files_runtime; // Phase 23.C — Lost Cargo file-handler orchestration lifted from commands/community/files.rs.
pub mod community_link_previews_runtime; // Phase 23.C — link-preview settings SQLite ops lifted from commands/community/link_previews.rs.
pub mod community_mek_local_rotate; // Phase 23.C — `rotate_mek_local` lifted from legacy/control.rs.
pub mod community_moderation_bulk; // Phase 23.D.4 — bulk channel-message delete extracted from community_moderation_runtime.
pub mod community_moderation_runtime; // Phase 23.C — message-deletion runtime orchestration lifted from commands/community/moderation.rs.
pub mod community_notifications_runtime; // Phase 23.C — quiet-hours wrappers lifted from commands/community/notifications.rs.
pub mod community_onboarding_mappers; // Phase 23.C — pure protocol↔governance onboarding DTO converters lifted from legacy/onboarding.rs.
pub mod community_onboarding_runtime; // Phase 23.C — onboarding-handler runtime orchestration lifted from commands/community/onboarding.rs.
pub mod community_onboarding_validation; // Phase 23.C — pure onboarding validator lifted from commands/community/onboarding.rs.
pub mod community_pins_runtime; // Phase 23.C — pin/unpin/get_channel_pins orchestration lifted from commands/community/reactions_pins.rs.
pub mod community_presence_runtime; // Phase 23.C — presence-handler runtime orchestration lifted from commands/community/presence.rs.
pub mod community_unread_runtime; // Phase 23.C — unread mark/get bodies lifted from commands/community/unread.rs.
pub mod community_video_runtime; // Phase 23.C — video-handler runtime orchestration lifted from commands/community/video.rs.
pub mod community_views_runtime; // Phase 23.C — community list/detail DTO mappers lifted from commands/community/crud.rs.
pub mod cross_device_sync;
pub mod dm;
pub mod dm_runtime; // Phase 23.C — DM video-frame command orchestration lifted from commands/dm.rs.
pub mod files_adapter; // Phase 15 — FilesDeps impl + Tier-9 facades.
pub mod gossip_adapter; // Phase 20.e-REDO — GossipDeps impl for mesh broadcast.
pub mod mek_adapter; // Phase 17 — MekDistributeDeps impl + ChannelMekCache wrapper + Stronghold persist.
pub mod message_service;
pub mod messaging_runtime; // Phase 23.C — channel-messaging runtime orchestration lifted from commands/community/messaging.rs.
pub mod native_video; // Linux-native GStreamer camera capture facade (plan rosy-bouncing-kahn Phase 3).
pub mod preferences; // Plan D1 — the unlocked identity's preferences.
pub mod presence_adapter; // Phase 21.e-REDO — FriendPresenceDeps impl for friend presence.
pub mod push_relay;
pub mod relay;
pub mod search;
pub mod shell_session; // Plan E1.1 — the GUI's ShellHooks: its unported unlock stages.
pub mod sync_communities;
pub mod sync_runtime; // Phase 23.C — sync-handler runtime orchestration lifted from commands/sync.rs.
pub mod sync_service;
pub mod veilid;
pub mod video_adapter; // Phase 16 — VideoDeps impl + send/receive facades.
pub mod voice_adapter; // Phase 14 — VoiceSessionDeps impl + voice free-fn facades.
pub mod voice_runtime; // Phase 23.C — voice auxiliary helpers (audio device list/prefs + stage audience gate).
pub mod voice_signaling_adapter; // Phase 14.k — VoiceSignalingDeps impl + signaling facade.
pub mod window_runtime; // Phase 23.C — window helpers (get_network_status body).

use std::sync::Arc;

use crate::state::AppState;
use rekindle_db::Db;

/// Build a service adapter from the live app context.
///
/// Every adapter in this module takes the same three things —
/// `Arc<AppState>`, the `AppHandle` and the `Db` — so every callsite
/// grew its own local `build_adapter` to fetch them. Fifteen of those
/// accumulated, and six were byte-identical in two groups of three that
/// differed *only* in the message they returned for the same failure:
/// "app handle or Db unavailable" against "app handle not
/// initialized". That is what this duplication actually cost — not the
/// lines, but two names for one condition, which is how a caller ends up
/// matching on the wrong one.
///
/// Passing the constructor as a `fn` pointer rather than introducing a
/// trait keeps this to one function: every adapter's `new` already has
/// this exact shape, whether it returns `Self` or `Arc<Self>`.
pub fn build_adapter<A>(
    state: &Arc<AppState>,
    new: fn(Arc<AppState>, tauri::AppHandle, Db) -> A,
) -> Result<A, String> {
    let (app_handle, pool) = crate::state_helpers::app_context(state)
        .ok_or_else(|| "app handle or Db unavailable".to_string())?;
    Ok(new(Arc::clone(state), app_handle, pool))
}

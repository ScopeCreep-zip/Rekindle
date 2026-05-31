//! Phase 14.l — `start_session` orchestrator port.
//!
//! Pre-Phase-14.l this lived in `src-tauri/services/voice/session.rs`
//! (693 LoC, deeply coupled to AppState). The orchestration sequence
//! is unchanged from the original; AppState mutation moved behind
//! `VoiceSessionDeps` methods (`init_voice_session`,
//! `spawn_voice_loops`, etc.) implemented by `VoiceAdapter`.

use std::sync::Arc;

use crate::error::VoiceError;
use crate::session_deps::VoiceSessionDeps;

/// Bring up a voice session.
///
/// Order of operations (architecture §10.4 voice pipeline + W14.1
/// pre-stage + W13.12 1:1 route fallback + W13.14 AEAD call_key
/// install + §10.6 capabilities broadcast):
///
/// 1. Verify we're not already in a different voice channel.
/// 2. Read identity (public_key + display_name) for join events.
/// 3. Load audio preferences from the Tauri store.
/// 4. For 1:1 calls (community_id == None): resolve the peer's
///    route via the W13.12 fallback chain (cache → DHT subkey 6 →
///    mailbox). Fail closed if all three empty.
/// 5. `deps.init_voice_session(...)`: build the VoiceEngine, start
///    cpal devices, build the transport, install the call_key for
///    1:1, store the engine handle on AppState. Returns the
///    (muted_flag, deafened_flag, transport) tuple.
/// 6. Load community member display names from SQLite (empty map
///    for 1:1 calls).
/// 7. `deps.spawn_voice_loops(...)`: spawn send / receive / device
///    monitor loops, register handles on the engine for shutdown.
/// 8. Emit `VoiceEvent::LocalJoined` so frontends transition to
///    in-call UI.
/// 9. For community channels: broadcast `MediaCapabilities` so
///    other senders cap their VP9 bitrate at the lowest common
///    denominator (§10.6).
pub async fn start_session<D: VoiceSessionDeps + ?Sized>(
    deps: &Arc<D>,
    channel_id: &str,
    community_id: Option<&str>,
) -> Result<(), VoiceError> {
    deps.check_not_in_call(channel_id)?;

    let identity = deps.current_identity()?;
    let prefs = deps.audio_prefs();

    // W13.12 — 1:1 voice needs a resolved peer route BEFORE the
    // transport is initialized. If all three fallback sources (cache,
    // DHT subkey 6, mailbox) are empty, fail closed so the caller's
    // accept_dm_call / handle_accept_received can convert to a
    // CallDecline / CallEnd. We never silently transport.init()
    // without a peer.
    let peer_route = if community_id.is_none() {
        match deps.resolve_peer_route(channel_id).await {
            Some(blob) => Some(blob),
            None => {
                return Err(VoiceError::Session(format!(
                    "no route to peer {channel_id} (cache + DHT + mailbox all empty)"
                )));
            }
        }
    } else {
        None
    };

    let startup = deps.init_voice_session(
        &prefs,
        channel_id,
        community_id,
        peer_route.as_deref(),
    )?;

    let member_names = deps.load_member_names(community_id).await;

    deps.spawn_voice_loops(
        &identity.public_key,
        startup.transport,
        startup.muted_flag,
        startup.deafened_flag,
        member_names,
    )?;

    deps.emit_local_joined(
        channel_id,
        community_id,
        &identity.public_key,
        &identity.display_name,
    );

    if let Some(community_id) = community_id {
        deps.broadcast_media_capabilities(community_id, channel_id);
    }

    Ok(())
}

//! Architecture §10.6 Phase B — per-call video session aggregation.
//!
//! Holds per-(community, channel) `MediaCapabilities` from every peer
//! currently in the call (including the local peer). On every
//! membership change or cap-receipt the room-wide `SessionVideoConfig`
//! is recomputed via `rekindle_video::negotiate_session_config` and,
//! if it differs from the last value the backend emitted, re-broadcast
//! through `CommunityEvent::VideoSessionConfig`.
//!
//! Backend owns the policy: CLI / TUI / Tauri frontends all consume
//! the same event and inherit the same encoder + decoder constraints
//! without ever running their own intersection logic. This module
//! sits in `services/community/` (the Tauri-shell frontend's runtime),
//! not in a tier crate, because aggregating per-(community, channel)
//! state across IPC + gossip + voice-session signals is intrinsically
//! a shell-runtime concern. The negotiator itself stays pure in
//! `rekindle_video::policy`.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use rekindle_video::{negotiate_session_config, MediaCapabilities, SessionVideoConfig};

use crate::channels::CommunityEvent;
use crate::state::AppState;

/// Stable key the local peer is registered under in `peer_caps`. Using
/// a non-hex literal that cannot collide with a real Ed25519 pseudonym
/// hex (64 chars) lets `on_local_caps_reported` populate the local
/// entry across every active session without needing the local
/// pseudonym lookup at the call site (`on_local_caps_reported` is
/// invoked before the first `on_local_joined` from the WebView startup
/// probe).
pub(crate) const LOCAL_PEER_KEY: &str = "<local>";

/// Single (community, channel) call's aggregated capability state.
///
/// `peer_caps` is keyed by peer pseudonym hex (or `LOCAL_PEER_KEY` for
/// the local peer's reported caps). `last_emitted` tracks the most
/// recently broadcast `SessionVideoConfig` so we can elide redundant
/// re-emits when membership churn produces the same negotiated shape.
#[derive(Debug, Default, Clone)]
pub struct VideoSessionState {
    pub peer_caps: HashMap<String, MediaCapabilities>,
    pub last_emitted: Option<SessionVideoConfig>,
    /// Phase 3 latch — true after `VideoCodecIncompatible` was emitted
    /// for the current peer set. Prevents per-recompute toast spam:
    /// the event fires only on the compatible→incompatible TRANSITION,
    /// and a recovery (negotiation succeeds again) clears the latch and
    /// force-emits the config.
    pub last_incompatible: bool,
    /// Phase 3 — decode-only local (empty `encode_codecs`) is a
    /// supported mode, logged once per session instead of evented.
    pub decode_only_logged: bool,
}

/// Per-(community_id, channel_id) session state. `RwLock` is `parking_lot`
/// (matches the rest of `AppState`) — every helper acquires the write
/// guard, mutates a single `(community, channel)` slot, then drops the
/// guard before emitting so the emit path never holds the lock.
pub struct VideoSessionStateMap {
    inner: RwLock<HashMap<(String, String), VideoSessionState>>,
}

impl VideoSessionStateMap {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for VideoSessionStateMap {
    fn default() -> Self {
        Self::new()
    }
}

/// Outcome of a recompute pass. `Emit(config)` means the negotiated
/// shape changed (or this was a force-emit on `on_local_joined`) and the
/// caller should fan the new config out through `CommunityEvent::VideoSessionConfig`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RecomputeOutcome {
    /// The new config matches `last_emitted` (and force was not set).
    /// No emit — frontend already holds the current value.
    NoEmit,
    /// Emit `CommunityEvent::VideoSessionConfig { config }`. Includes
    /// the first-emit case and every change after.
    Emit(SessionVideoConfig),
    /// Phase 3 — no local encode codec is decodable by every peer.
    /// Emit `CommunityEvent::VideoCodecIncompatible { peers }` (only
    /// returned on the compatible→incompatible transition — latched).
    EmitIncompatible { peers: Vec<String> },
}

/// Core recompute + decide-to-emit pass. Splits the local caps
/// (`LOCAL_PEER_KEY`, falling back to the pending probe slot — covers a
/// peer's caps arriving via `ensure_slot` before `on_local_joined` ran)
/// from the remote peers', runs the pure-logic per-node negotiator, and
/// returns whether the caller should emit. `force_emit_unchanged=true`
/// is the load-bearing primitive for `on_local_joined`: a late-mounting
/// frontend must receive the current config even when the negotiated
/// shape happens to match the previous one (otherwise it would render
/// a black tile because no event ever arrives).
fn recompute_and_decide(
    sessions: &VideoSessionStateMap,
    community_id: &str,
    channel_id: &str,
    force_emit_unchanged: bool,
) -> Result<RecomputeOutcome, String> {
    let key = (community_id.to_string(), channel_id.to_string());
    let mut guard = sessions.inner.write();
    let pending_local = guard
        .get(&pending_slot_key())
        .and_then(|s| s.peer_caps.get(LOCAL_PEER_KEY).cloned());
    let entry = guard.get_mut(&key).ok_or_else(|| {
        format!(
            "video session state missing for community={community_id} channel={channel_id} — \
             programmer error: helper called before on_local_joined / on_peer_joined seeded \
             the slot"
        )
    })?;

    let local = entry
        .peer_caps
        .get(LOCAL_PEER_KEY)
        .cloned()
        .or(pending_local)
        .unwrap_or_else(MediaCapabilities::interim_default);
    let peers: Vec<MediaCapabilities> = entry
        .peer_caps
        .iter()
        .filter(|(k, _)| *k != LOCAL_PEER_KEY)
        .map(|(_, v)| v.clone())
        .collect();

    match negotiate_session_config(&local, &peers) {
        Some(new_config) => {
            entry.decode_only_logged = false;
            let was_incompatible = std::mem::take(&mut entry.last_incompatible);
            let unchanged = entry.last_emitted.as_ref() == Some(&new_config);
            // Recovering from incompatible force-emits even when the
            // shape matches: the frontend tore its session down on the
            // incompatible event and needs the config to restart.
            if unchanged && !force_emit_unchanged && !was_incompatible {
                return Ok(RecomputeOutcome::NoEmit);
            }
            entry.last_emitted = Some(new_config.clone());
            Ok(RecomputeOutcome::Emit(new_config))
        }
        None if local.encode_codecs.is_empty() => {
            // Decode-only platform — a supported mode (the user can
            // watch, not send). Not an incompatibility; log once.
            if !entry.decode_only_logged {
                entry.decode_only_logged = true;
                tracing::info!(
                    community_id,
                    channel_id,
                    "local WebView reports no video encoder — decode-only session"
                );
            }
            Ok(RecomputeOutcome::NoEmit)
        }
        None => {
            if entry.last_incompatible {
                return Ok(RecomputeOutcome::NoEmit);
            }
            entry.last_incompatible = true;
            // Name the individually-blocking peers (decode set disjoint
            // from our entire encode set); if the failure is only
            // collective (each candidate blocked by a different peer),
            // every peer is implicated.
            let blocking: Vec<String> = entry
                .peer_caps
                .iter()
                .filter(|(k, _)| *k != LOCAL_PEER_KEY)
                .filter(|(_, p)| {
                    local
                        .encode_codecs
                        .iter()
                        .all(|c| !p.decode_codecs.contains(c))
                })
                .map(|(k, _)| k.clone())
                .collect();
            let peers = if blocking.is_empty() {
                entry
                    .peer_caps
                    .keys()
                    .filter(|k| *k != LOCAL_PEER_KEY)
                    .cloned()
                    .collect()
            } else {
                blocking
            };
            Ok(RecomputeOutcome::EmitIncompatible { peers })
        }
    }
}

/// Fan a recompute outcome out to the frontend event stream.
fn apply_outcome(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    outcome: RecomputeOutcome,
) {
    match outcome {
        RecomputeOutcome::NoEmit => {}
        RecomputeOutcome::Emit(config) => {
            emit_session_config(state, community_id, channel_id, config);
            // Media-ready input: the frontend encoder now has a
            // negotiated shape to configure from.
            crate::services::community::media_ready_runtime::update_media_ready(
                state,
                community_id,
                channel_id,
                |i| i.session_config_emitted = true,
            );
        }
        RecomputeOutcome::EmitIncompatible { peers } => {
            let event = CommunityEvent::VideoCodecIncompatible(
                crate::channels::VideoCodecIncompatibleEvent {
                    community_id: community_id.to_string(),
                    channel_id: channel_id.to_string(),
                    peers,
                },
            );
            crate::event_dispatch::emit_now(state, "community-event", &event);
        }
    }
}

/// Ensure the `(community, channel)` slot exists. Idempotent — called
/// from every entry point that may be the first observer of the call.
fn ensure_slot(sessions: &VideoSessionStateMap, community_id: &str, channel_id: &str) {
    let key = (community_id.to_string(), channel_id.to_string());
    let mut guard = sessions.inner.write();
    guard.entry(key).or_default();
}

/// Push a peer's caps (or the local peer's caps under `LOCAL_PEER_KEY`)
/// into the slot. Slot must already exist (`ensure_slot` first).
fn insert_caps(
    sessions: &VideoSessionStateMap,
    community_id: &str,
    channel_id: &str,
    peer_key: &str,
    caps: MediaCapabilities,
) -> Result<(), String> {
    let key = (community_id.to_string(), channel_id.to_string());
    let mut guard = sessions.inner.write();
    let entry = guard.get_mut(&key).ok_or_else(|| {
        format!(
            "video session state missing for community={community_id} channel={channel_id} — \
             programmer error: insert_caps called without ensure_slot"
        )
    })?;
    entry.peer_caps.insert(peer_key.to_string(), caps);
    Ok(())
}

/// Drop a peer's caps from the slot. Slot must already exist. Returns
/// `Err` if the slot is missing — that's a programmer error (the leave
/// path was triggered without ever observing a join).
fn remove_caps(
    sessions: &VideoSessionStateMap,
    community_id: &str,
    channel_id: &str,
    peer_key: &str,
) -> Result<(), String> {
    let key = (community_id.to_string(), channel_id.to_string());
    let mut guard = sessions.inner.write();
    let entry = guard.get_mut(&key).ok_or_else(|| {
        format!(
            "video session state missing for community={community_id} channel={channel_id} — \
             programmer error: remove_caps called without ensure_slot"
        )
    })?;
    entry.peer_caps.remove(peer_key);
    Ok(())
}

/// Emit `CommunityEvent::VideoSessionConfig` through the central
/// dispatch queue. Uses `emit_now` so the call site doesn't need an
/// `AppHandle` — every caller already has `&Arc<AppState>`. The
/// dispatch loop forwards through Tauri exactly like the other
/// community-event variants.
fn emit_session_config(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    config: SessionVideoConfig,
) {
    let event = CommunityEvent::VideoSessionConfig(crate::channels::VideoSessionConfigEvent {
        community_id: community_id.to_string(),
        channel_id: channel_id.to_string(),
        config,
    });
    crate::event_dispatch::emit_now(state, "community-event", &event);
}

/// Frontend (WebCodecs probe matrix) reported the WebView's real
/// encoder + decoder capability set. Replaces the placeholder caps
/// every active session is holding for the local peer and recomputes
/// each one — a single probe result may change the negotiated shape
/// for multiple in-progress calls simultaneously. Cached so a
/// subsequent `on_local_joined` (new call) reuses the same caps
/// without forcing a second frontend round-trip.
pub fn on_local_caps_reported(
    state: &Arc<AppState>,
    caps: MediaCapabilities,
) -> Result<(), String> {
    let slots: Vec<(String, String)> = {
        let guard = state.video_sessions.inner.read();
        guard
            .keys()
            .filter(|k| *k != &pending_slot_key())
            .cloned()
            .collect()
    };
    for (community_id, channel_id) in &slots {
        insert_caps(
            &state.video_sessions,
            community_id,
            channel_id,
            LOCAL_PEER_KEY,
            caps.clone(),
        )?;
        let outcome = recompute_and_decide(&state.video_sessions, community_id, channel_id, false)?;
        apply_outcome(state, community_id, channel_id, outcome);
        // Media-ready input: a join that raced the probe seeded
        // `local_caps_reported = false` and NOTHING else ever flipped
        // it — the gate wedged at `caps-unreported` for the whole
        // session (field-observed: joining within ~40s of app start).
        // The probe landing IS the "reported" edge; flip it on every
        // active session slot.
        crate::services::community::media_ready_runtime::update_media_ready(
            state,
            community_id,
            channel_id,
            |i| i.local_caps_reported = true,
        );
    }
    set_pending_local_caps(state, Some(caps));
    Ok(())
}

/// Sentinel slot key the frontend's pre-call WebCodecs probe writes to
/// when no community/channel call is active yet. `on_local_joined` reads
/// from it to seed the real call slot with the strongest caps reported
/// so far. Kept in `VideoSessionStateMap` rather than a sibling `AppState`
/// field so we don't grow the (already large) `AppState` struct for a
/// single Option.
fn pending_slot_key() -> (String, String) {
    ("<pending>".to_string(), "<pending>".to_string())
}

fn set_pending_local_caps(state: &Arc<AppState>, caps: Option<MediaCapabilities>) {
    let mut guard = state.video_sessions.inner.write();
    let entry = guard.entry(pending_slot_key()).or_default();
    entry.peer_caps.clear();
    if let Some(caps) = caps {
        entry.peer_caps.insert(LOCAL_PEER_KEY.to_string(), caps);
    }
}

/// The most recently reported local WebCodecs probe caps, or `None`
/// if the frontend hasn't reported yet. Also feeds the directed
/// `MediaCapabilities` advertisements (`voice_adapter::io_helpers`) so
/// peers negotiate against our real probe matrix, not the placeholder.
pub fn reported_local_caps(state: &Arc<AppState>) -> Option<MediaCapabilities> {
    let guard = state.video_sessions.inner.read();
    guard
        .get(&pending_slot_key())
        .and_then(|s| s.peer_caps.get(LOCAL_PEER_KEY).cloned())
}

fn pending_local_caps(state: &Arc<AppState>) -> MediaCapabilities {
    reported_local_caps(state).unwrap_or_else(MediaCapabilities::interim_default)
}

/// The local peer joined a (community, channel) voice/video session.
/// Seeds the slot with the most recently reported local caps (falling
/// back to `MediaCapabilities::interim_default()` if the frontend
/// probe hasn't run yet) and force-emits the negotiated config so a
/// late-mounting frontend doesn't sit on a black tile waiting for a
/// delta event that will never come.
pub fn on_local_joined(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
) -> Result<(), String> {
    ensure_slot(&state.video_sessions, community_id, channel_id);
    let local_caps = pending_local_caps(state);
    insert_caps(
        &state.video_sessions,
        community_id,
        channel_id,
        LOCAL_PEER_KEY,
        local_caps,
    )?;
    let outcome = recompute_and_decide(&state.video_sessions, community_id, channel_id, true)?;
    apply_outcome(state, community_id, channel_id, outcome);
    Ok(())
}

/// A remote peer joined the call. We don't yet know their real caps
/// (they'll be advertised via `MediaCapabilities` gossip on join);
/// seed with `optimistic_peer_default` — listing every codec we ship —
/// so a pre-caps joiner never blocks the local encoder pick. The
/// placeholder self-heals when the real advertisement lands
/// (`on_peer_caps_received` recomputes), and receivers create decoders
/// from per-fragment tags regardless of what this node encodes.
pub fn on_peer_joined(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    sender_pseudonym: &str,
) -> Result<(), String> {
    ensure_slot(&state.video_sessions, community_id, channel_id);
    insert_caps(
        &state.video_sessions,
        community_id,
        channel_id,
        sender_pseudonym,
        MediaCapabilities::optimistic_peer_default(),
    )?;
    let outcome = recompute_and_decide(&state.video_sessions, community_id, channel_id, false)?;
    apply_outcome(state, community_id, channel_id, outcome);
    Ok(())
}

/// A peer left the call. Drop their caps and recompute — the
/// negotiator may now pick a stronger config the departed peer was
/// holding everyone back from.
pub fn on_peer_left(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    sender_pseudonym: &str,
) -> Result<(), String> {
    remove_caps(
        &state.video_sessions,
        community_id,
        channel_id,
        sender_pseudonym,
    )?;
    let outcome = recompute_and_decide(&state.video_sessions, community_id, channel_id, false)?;
    apply_outcome(state, community_id, channel_id, outcome);
    Ok(())
}

/// A peer's gossiped `MediaCapabilities` envelope was received and
/// decoded. Replace their entry in the slot (idempotent — multiple
/// advertisements just overwrite the same key) and recompute.
pub fn on_peer_caps_received(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    sender_pseudonym: &str,
    caps: MediaCapabilities,
) -> Result<(), String> {
    ensure_slot(&state.video_sessions, community_id, channel_id);
    insert_caps(
        &state.video_sessions,
        community_id,
        channel_id,
        sender_pseudonym,
        caps,
    )?;
    let outcome = recompute_and_decide(&state.video_sessions, community_id, channel_id, false)?;
    apply_outcome(state, community_id, channel_id, outcome);
    Ok(())
}

#[cfg(test)]
#[path = "video_session/tests.rs"]
mod tests;

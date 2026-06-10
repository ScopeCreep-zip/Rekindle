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
        }
        RecomputeOutcome::EmitIncompatible { peers } => {
            let event = CommunityEvent::VideoCodecIncompatible {
                community_id: community_id.to_string(),
                channel_id: channel_id.to_string(),
                peers,
            };
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
    let event = CommunityEvent::VideoSessionConfig {
        community_id: community_id.to_string(),
        channel_id: channel_id.to_string(),
        config,
    };
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
mod tests {
    //! Integration tests for the Phase B aggregation loop. Each scenario
    //! constructs a stand-alone `AppState`, drives the public helpers,
    //! and asserts on `last_emitted` because verifying actual `app.emit`
    //! requires a Tauri `AppHandle` we cannot construct in unit tests.
    //!
    //! The same code path that updates `last_emitted` is the one that
    //! enqueues the emit (`emit_session_config` runs unconditionally in
    //! the `Emit` branch), so `last_emitted` is the load-bearing
    //! invariant Phase B promised.

    use super::*;
    use rekindle_types::video::{Codec, ScalabilityMode};

    const COMMUNITY_ID: &str = "comm_1";
    const CHANNEL_ID: &str = "ch_1";

    fn fresh_state() -> Arc<AppState> {
        Arc::new(AppState::default())
    }

    fn snapshot(state: &Arc<AppState>) -> Option<SessionVideoConfig> {
        let guard = state.video_sessions.inner.read();
        guard
            .get(&(COMMUNITY_ID.to_string(), CHANNEL_ID.to_string()))
            .and_then(|s| s.last_emitted.clone())
    }

    fn weaker_caps() -> MediaCapabilities {
        // 320×240 @ 10fps — strictly weaker than `interim_default`
        // (854×480 @ 15fps). Forces a different negotiated shape.
        MediaCapabilities {
            max_pixel_count: 320 * 240,
            max_fps: 10,
            ..MediaCapabilities::interim_default()
        }
    }

    /// Phase 3 latch state for the test slot.
    fn incompatible_latch(state: &Arc<AppState>) -> bool {
        let guard = state.video_sessions.inner.read();
        guard
            .get(&(COMMUNITY_ID.to_string(), CHANNEL_ID.to_string()))
            .is_some_and(|s| s.last_incompatible)
    }

    /// A peer that decodes ONLY H.264 — disjoint from the local
    /// interim default's VP9-only encode set.
    fn h264_only_decoder() -> MediaCapabilities {
        MediaCapabilities {
            decode_codecs: vec![Codec::H264],
            ..MediaCapabilities::interim_default()
        }
    }

    #[test]
    fn local_joined_emits_initial_config() {
        let state = fresh_state();
        assert!(snapshot(&state).is_none());
        on_local_joined(&state, COMMUNITY_ID, CHANNEL_ID).expect("on_local_joined");
        let cfg = snapshot(&state).expect("first emit must populate last_emitted");
        // Local-only call resolves to the interim default shape.
        let baseline = negotiate_session_config(&MediaCapabilities::interim_default(), &[])
            .expect("local-only negotiation succeeds");
        assert_eq!(cfg, baseline, "local-only config = interim_default");
    }

    #[test]
    fn weaker_peer_join_triggers_new_emit() {
        let state = fresh_state();
        on_local_joined(&state, COMMUNITY_ID, CHANNEL_ID).unwrap();
        let before = snapshot(&state).unwrap();

        // Inject the weaker caps via the cap-receipt path so the slot
        // holds the real peer caps (not the interim_default placeholder
        // that on_peer_joined writes).
        on_peer_joined(&state, COMMUNITY_ID, CHANNEL_ID, "peer_weak").unwrap();
        on_peer_caps_received(&state, COMMUNITY_ID, CHANNEL_ID, "peer_weak", weaker_caps())
            .unwrap();

        let after = snapshot(&state).expect("after weaker peer join");
        assert_ne!(
            before, after,
            "negotiated config must change when a strictly weaker peer joins"
        );
        let expected =
            negotiate_session_config(&MediaCapabilities::interim_default(), &[weaker_caps()])
                .expect("compatible");
        assert_eq!(after, expected);
    }

    #[test]
    fn peer_left_reverts_to_local_only_emit() {
        let state = fresh_state();
        on_local_joined(&state, COMMUNITY_ID, CHANNEL_ID).unwrap();
        on_peer_joined(&state, COMMUNITY_ID, CHANNEL_ID, "peer_weak").unwrap();
        on_peer_caps_received(&state, COMMUNITY_ID, CHANNEL_ID, "peer_weak", weaker_caps())
            .unwrap();

        on_peer_left(&state, COMMUNITY_ID, CHANNEL_ID, "peer_weak").unwrap();
        let after = snapshot(&state).unwrap();
        let baseline = negotiate_session_config(&MediaCapabilities::interim_default(), &[])
            .expect("local-only negotiation succeeds");
        assert_eq!(after, baseline, "leave reverts to local-only config");
    }

    #[test]
    fn matching_caps_emit_is_idempotent() {
        let state = fresh_state();
        on_local_joined(&state, COMMUNITY_ID, CHANNEL_ID).unwrap();
        let before = snapshot(&state).unwrap();

        // A peer joins whose caps match interim_default — the
        // negotiated shape doesn't change, so the helper must NOT emit
        // (idempotent invariant). We observe this by checking that
        // `last_emitted` is unchanged value-wise after the join.
        on_peer_joined(&state, COMMUNITY_ID, CHANNEL_ID, "peer_mirror").unwrap();
        on_peer_caps_received(
            &state,
            COMMUNITY_ID,
            CHANNEL_ID,
            "peer_mirror",
            MediaCapabilities::interim_default(),
        )
        .unwrap();
        let after = snapshot(&state).unwrap();
        assert_eq!(
            before, after,
            "idempotent: matching caps must produce the same negotiated config"
        );
    }

    #[test]
    fn local_caps_replace_interim_default() {
        let state = fresh_state();
        // Frontend reports a stronger probe BEFORE any call is active.
        let strong = MediaCapabilities {
            max_pixel_count: 1280 * 720,
            max_fps: 30,
            supports_optimize_for_latency: true,
            supported_scalability_modes: vec![ScalabilityMode::Flat, ScalabilityMode::L1T2],
            ..MediaCapabilities::interim_default()
        };
        on_local_caps_reported(&state, strong.clone()).unwrap();
        // No active call → nothing to emit yet but the caps are cached.
        assert!(snapshot(&state).is_none());

        on_local_joined(&state, COMMUNITY_ID, CHANNEL_ID).unwrap();
        let cfg = snapshot(&state).unwrap();
        // Local-only session should pick up the stronger caps, not the
        // interim default.
        let expected = negotiate_session_config(&strong, &[]).expect("local-only");
        assert_eq!(cfg, expected);
    }

    #[test]
    fn incompatible_peer_latches_once_and_recovers() {
        let state = fresh_state();
        on_local_joined(&state, COMMUNITY_ID, CHANNEL_ID).unwrap();
        let before = snapshot(&state).unwrap();
        assert!(!incompatible_latch(&state));

        // Real caps arrive: the peer decodes only H.264 while the local
        // interim default encodes only VP9 → empty intersection.
        on_peer_joined(&state, COMMUNITY_ID, CHANNEL_ID, "peer_h264").unwrap();
        on_peer_caps_received(
            &state,
            COMMUNITY_ID,
            CHANNEL_ID,
            "peer_h264",
            h264_only_decoder(),
        )
        .unwrap();
        assert!(
            incompatible_latch(&state),
            "disjoint codec sets must latch incompatible"
        );
        assert_eq!(
            snapshot(&state).as_ref(),
            Some(&before),
            "incompatible negotiation must not clobber the last emitted config"
        );

        // Membership churn while incompatible: latch stays set (no
        // second EmitIncompatible — verified structurally by the latch
        // short-circuit in recompute_and_decide).
        on_peer_caps_received(
            &state,
            COMMUNITY_ID,
            CHANNEL_ID,
            "peer_h264",
            h264_only_decoder(),
        )
        .unwrap();
        assert!(incompatible_latch(&state));

        // Recovery: the peer re-advertises with VP9 decode → latch
        // clears and the config force-emits.
        on_peer_caps_received(
            &state,
            COMMUNITY_ID,
            CHANNEL_ID,
            "peer_h264",
            MediaCapabilities::interim_default(),
        )
        .unwrap();
        assert!(!incompatible_latch(&state), "recovery must clear the latch");
        assert_eq!(snapshot(&state).as_ref(), Some(&before));
    }

    #[test]
    fn decode_only_local_is_not_incompatible() {
        let state = fresh_state();
        // Local WebView probe found no encoder at all.
        let decode_only = MediaCapabilities {
            encode_codecs: vec![],
            ..MediaCapabilities::interim_default()
        };
        on_local_caps_reported(&state, decode_only).unwrap();
        on_local_joined(&state, COMMUNITY_ID, CHANNEL_ID).unwrap();
        assert!(
            snapshot(&state).is_none(),
            "decode-only local has no encoder config to emit"
        );
        assert!(
            !incompatible_latch(&state),
            "decode-only is a supported mode, not an incompatibility"
        );
    }

    #[test]
    fn pre_caps_peer_placeholder_never_blocks() {
        let state = fresh_state();
        on_local_joined(&state, COMMUNITY_ID, CHANNEL_ID).unwrap();
        // A joiner whose advertisement hasn't landed yet must not flip
        // the session incompatible (optimistic placeholder seeds every
        // shipped codec).
        on_peer_joined(&state, COMMUNITY_ID, CHANNEL_ID, "peer_pending").unwrap();
        assert!(!incompatible_latch(&state));
        assert!(snapshot(&state).is_some());
    }

    #[test]
    fn helpers_on_missing_slot_error_loudly() {
        let state = fresh_state();
        // `on_peer_left` against a slot that was never seeded must
        // return Err — silently creating + emitting would mask the
        // bug that produced the spurious leave event.
        let err = on_peer_left(&state, COMMUNITY_ID, CHANNEL_ID, "ghost")
            .expect_err("peer_left on missing slot must error");
        assert!(
            err.contains("video session state missing"),
            "error should name the invariant: got {err}"
        );
    }
}

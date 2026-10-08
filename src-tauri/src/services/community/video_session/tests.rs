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
    on_peer_caps_received(&state, COMMUNITY_ID, CHANNEL_ID, "peer_weak", weaker_caps()).unwrap();

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
    on_peer_caps_received(&state, COMMUNITY_ID, CHANNEL_ID, "peer_weak", weaker_caps()).unwrap();

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

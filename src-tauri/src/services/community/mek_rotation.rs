//! A delivered key (`MekTransfer`) is received by the Host
//! (`HostGovernance::receive_mek_transfer`, plan E3.1 CB3 glue audit), never
//! here. Only `spawn_mek_request_with_retry` stays here — it's a Tier-9 tokio::spawn wrapper that pushes
//! `RequestMEK` envelopes onto the mesh on a cascade-fall-through schedule.

use std::sync::Arc;

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_types::channel_keys::KeyScope;

use crate::state::AppState;

/// Ask the mesh for `scope`'s key at `needed_generation` (0 = "your
/// current"), cascading through responders until a satisfying key for
/// exactly that scope lands.
pub fn spawn_mek_request_with_retry(
    state: Arc<AppState>,
    community_id: String,
    scope: KeyScope,
    needed_generation: u64,
    requester_pseudonym: String,
) {
    let channel_id = scope.wire_channel_str();
    // Phase 17 — MAX_CASCADES sourced from the rekindle-mek-rotation
    // crate so the requester-side retry budget stays in lock-step with
    // the rotator-side cascade_candidates(max_cascades) ceiling. The
    // spawn task itself stays src-tauri-local (tokio::spawn against
    // AppState; not crate-side protocol logic).
    crate::state_helpers::spawn_in_login_with_token(
        &state.clone(),
        "MEK request retry",
        |stop| async move {
            let max_cascades = u32::try_from(rekindle_mek_rotation::MAX_CASCADES).unwrap_or(3);
            const RETRY_DEADLINE_MS: u64 = 5_000;
            // Snapshot the resolution at spawn so `0` ("send me current")
            // can detect that ANY new key landed.
            let resolve = || {
                crate::state_helpers::current_mek(&state, &community_id, scope)
                    .map(|mek| (*mek.as_bytes(), mek.generation()))
            };
            let initial = resolve();
            let initial_gen = initial.map(|(_, generation)| generation);

            // Did we ALREADY hold something that satisfies this request
            // when it was made?
            //
            // If so, the caller is not telling us a generation is missing —
            // it is telling us the bytes we hold at that generation do not
            // work. That is the channel-MEK split-brain: two peers minted
            // different keys at the same generation, so both sides "have
            // generation N" and neither can decrypt the other.
            //
            // This distinction is load-bearing. Without it the loop below
            // sees `current >= needed`, calls it a cache hit, and returns
            // before sending anything — so the one situation that most
            // needs a request is the one that never sends one. Observed
            // live as 625 consecutive `video frame MEK decrypt failed at
            // matching generation` warnings with zero RequestMEK sent.
            // `needed_generation != 0`: "send me your current" is never
            // satisfied up front — the whole point is to learn whether a
            // newer key exists.
            let already_satisfied_at_spawn = needed_generation != 0
                && initial_gen.is_some_and(|generation| generation >= needed_generation);
            for cascade_index in 0..max_cascades {
                // Bail early if a satisfying MEK arrived via a concurrent
                // path (parallel rotation broadcast, an MekTransfer reply,
                // a different request that produced the same gen).
                // Satisfied means: resolution at/after the needed
                // generation — the responder may serve CURRENT when the
                // exact historical generation is gone, which still
                // converges the live stream.
                let resolved = resolve();
                let cache_hit = cascade_cache_hit(
                    needed_generation,
                    already_satisfied_at_spawn,
                    initial,
                    resolved,
                );
                if cache_hit {
                    return;
                }
                // Build & broadcast RequestMEK at the current cascade level.
                let request = CommunityEnvelope::control(ControlPayload::RequestMEK {
                    channel_id: channel_id.clone(),
                    needed_generation,
                    requester_pseudonym: requester_pseudonym.clone(),
                    cascade_index,
                });
                if let Err(e) = crate::state_helpers::host_governance(&state)
                    .map(|host| host.send_to_mesh_impl(&community_id, &request))
                    .map_err(|e| e.to_string())
                {
                    tracing::warn!(
                        community = %community_id,
                        channel = %channel_id,
                        cascade_index,
                        error = %e,
                        "RequestMEK broadcast failed — will retry at next cascade level"
                    );
                } else {
                    tracing::debug!(
                        community = %community_id,
                        channel = %channel_id,
                        needed_generation,
                        cascade_index,
                        "RequestMEK sent"
                    );
                }
                let wait = tokio::time::sleep(std::time::Duration::from_millis(RETRY_DEADLINE_MS));
                if stop.run_until_cancelled(wait).await.is_none() {
                    return;
                }
            }
            tracing::warn!(
                community = %community_id,
                channel = %channel_id,
                needed_generation,
                "MEK request gave up after MAX_CASCADES attempts — channel messages remain undecryptable until next rotation broadcast"
            );
        },
    );
}

/// Decide whether a cascade-retry iteration can stop sending `RequestMEK`
/// because a satisfying key has already landed.
///
/// `already_satisfied_at_spawn` captures the channel-MEK split-brain
/// case: if we already held something meeting `needed_generation` when
/// the request was made, the caller isn't missing a generation — the
/// bytes we hold at that generation don't decrypt for them, because two
/// peers minted different keys at the same generation. In that case only
/// a KEY CHANGE (not a generation bump) can end the retry: a plain
/// generation check would be satisfied by the exact key that's already
/// failing to decrypt. Pulled out of the retry loop as its own function so
/// it can be tested directly against the split-brain scenario rather than
/// only through the live cascade loop.
#[must_use]
fn cascade_cache_hit(
    needed_generation: u64,
    already_satisfied_at_spawn: bool,
    initial: Option<([u8; 32], u64)>,
    resolved: Option<([u8; 32], u64)>,
) -> bool {
    if already_satisfied_at_spawn {
        resolved.map(|(key, _)| key) != initial.map(|(key, _)| key) && resolved.is_some()
    } else {
        let initial_gen = initial.map(|(_, generation)| generation);
        let resolved_gen = resolved.map(|(_, generation)| generation);
        match (needed_generation, resolved_gen) {
            (0, current) => current != initial_gen && current.is_some(),
            (needed, Some(current)) => current >= needed,
            (_, None) => false,
        }
    }
}

/// Community-key catch-up (architecture §7.3): ask the mesh for the
/// current community key through the retrying `RequestMEK` cascade. It
/// never mints: a key the community has not agreed on is a fork, and only
/// a confirmed ADR 0018 ballot raises the key (FX3; plan E3.5 builds the
/// ballot). A member no peer answers stays on the keys it holds until a
/// holder is reachable (the Host's sync tick asks again).
pub fn spawn_community_mek_recovery(
    state: Arc<AppState>,
    community_id: String,
    requester_pseudonym: String,
) {
    spawn_mek_request_with_retry(
        state,
        community_id,
        KeyScope::Community,
        0,
        requester_pseudonym,
    );
}

#[cfg(test)]
mod tests {
    use super::cascade_cache_hit;

    const KEY_A: [u8; 32] = [1u8; 32];
    const KEY_B: [u8; 32] = [2u8; 32];

    #[test]
    fn split_brain_same_generation_different_key_is_not_a_hit_until_key_changes() {
        // The bug this guards against: we already hold generation 5
        // (KEY_A) and it's failing to decrypt — `needed_generation == 5`
        // looks "already satisfied" by generation alone. Re-resolving
        // the SAME key must not end the retry.
        assert!(!cascade_cache_hit(
            5,
            true,
            Some((KEY_A, 5)),
            Some((KEY_A, 5))
        ));
        // A different key landing at the same (or any) generation does.
        assert!(cascade_cache_hit(
            5,
            true,
            Some((KEY_A, 5)),
            Some((KEY_B, 5))
        ));
        assert!(cascade_cache_hit(
            5,
            true,
            Some((KEY_A, 5)),
            Some((KEY_B, 9))
        ));
        // Nothing resolved at all is never a hit.
        assert!(!cascade_cache_hit(5, true, Some((KEY_A, 5)), None));
    }

    #[test]
    fn needed_generation_zero_wants_any_change_from_initial() {
        // "send me your current" (needed_generation == 0) is satisfied
        // by ANY resolution that differs from what we started with —
        // never by re-resolving to the same thing, even the same key.
        assert!(!cascade_cache_hit(
            0,
            false,
            Some((KEY_A, 1)),
            Some((KEY_A, 1))
        ));
        assert!(cascade_cache_hit(
            0,
            false,
            Some((KEY_A, 1)),
            Some((KEY_A, 2))
        ));
        assert!(cascade_cache_hit(0, false, None, Some((KEY_A, 1))));
        assert!(!cascade_cache_hit(0, false, None, None));
    }

    #[test]
    fn explicit_generation_wants_at_or_above_needed() {
        assert!(cascade_cache_hit(5, false, None, Some((KEY_A, 5))));
        assert!(cascade_cache_hit(5, false, None, Some((KEY_A, 9))));
        assert!(!cascade_cache_hit(5, false, None, Some((KEY_A, 4))));
        assert!(!cascade_cache_hit(5, false, None, None));
    }
}

//! Phase 23.D.10 — thin facade. All MEK receive logic
//! (unwrap_received_mek + apply_received_mek + handle_incoming_mek_transfer
//! + mek_cache_has_generation) ported into `rekindle_mek_rotation::receive`
//! parameterised over `MekDistributeDeps`. Only `spawn_mek_request_with_retry`
//! stays here — it's a Tier-9 tokio::spawn wrapper that pushes
//! `RequestMEK` envelopes onto the mesh on a cascade-fall-through schedule.

use std::sync::Arc;

use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_secrets::rotator::select_rotator;
use rekindle_types::id::PseudonymKey;

use crate::state::AppState;

pub fn spawn_mek_request_with_retry(
    state: Arc<AppState>,
    community_id: String,
    channel_id: String,
    needed_generation: u64,
    requester_pseudonym: String,
) {
    // Phase 17 — MAX_CASCADES sourced from the rekindle-mek-rotation
    // crate so the requester-side retry budget stays in lock-step with
    // the rotator-side cascade_candidates(max_cascades) ceiling. The
    // spawn task itself stays src-tauri-local (tokio::spawn against
    // AppState; not crate-side protocol logic).
    tokio::spawn(async move {
        let max_cascades = u32::try_from(rekindle_mek_rotation::MAX_CASCADES).unwrap_or(3);
        const RETRY_DEADLINE_MS: u64 = 5_000;
        // Snapshot the resolution at spawn so `0` ("send me current")
        // can detect that ANY new key landed.
        let initial_gen =
            crate::state_helpers::channel_media_mek(&state, &community_id, &channel_id)
                .map(|(_, generation)| generation);
        for cascade_index in 0..max_cascades {
            // Bail early if a satisfying MEK arrived via a concurrent
            // path (parallel rotation broadcast, an MekTransfer reply,
            // a different request that produced the same gen).
            // Satisfied means: resolution at/after the needed
            // generation — the responder may serve CURRENT when the
            // exact historical generation is gone, which still
            // converges the live stream.
            let resolved =
                crate::state_helpers::channel_media_mek(&state, &community_id, &channel_id)
                    .map(|(_, generation)| generation);
            let cache_hit = match (needed_generation, resolved) {
                (0, current) => current != initial_gen && current.is_some(),
                (needed, Some(current)) => current >= needed,
                (_, None) => false,
            };
            if cache_hit {
                return;
            }
            // Build & broadcast RequestMEK at the current cascade level.
            let request = CommunityEnvelope::Control(ControlPayload::RequestMEK {
                channel_id: channel_id.clone(),
                needed_generation,
                requester_pseudonym: requester_pseudonym.clone(),
                cascade_index,
            });
            if let Err(e) = super::send_to_mesh(&state, &community_id, &request) {
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
            tokio::time::sleep(std::time::Duration::from_millis(RETRY_DEADLINE_MS)).await;
        }
        tracing::warn!(
            community = %community_id,
            channel = %channel_id,
            needed_generation,
            "MEK request gave up after MAX_CASCADES attempts — channel messages remain undecryptable until next rotation broadcast"
        );
    });
}

/// Decide whether a community-MEK recovery should fall back to a last-resort
/// local mint. Mint ONLY when we are the elected minter AND nothing landed
/// during the re-acquire window (the slot started empty and is still empty —
/// no peer served the canonical key). An unelected peer never mints; an
/// elected one that did receive a key (slot now populated) keeps the
/// canonical one.
#[must_use]
pub fn should_last_resort_mint(initial: Option<u64>, current: Option<u64>, elected: bool) -> bool {
    elected && initial.is_none() && current.is_none()
}

/// Are we the peer that mints when nobody served the key?
///
/// This gate used to be `registry_owner_keypair.is_some()` — "did I
/// create this community". Under `o_cnt: 0` that keypair authorizes no
/// write on the registry at all, so it had degenerated into a pure
/// creator privilege: if the creator never came back, the community's
/// media stayed permanently undecryptable. That is the single point of
/// failure v2.0 removed.
///
/// The replacement is the election every other MEK decision already
/// uses — [`select_rotator`] over `blake3(context || candidate)` — with
/// the zero context `rotate_mek_on_request` stamps its provenance rank
/// against, so the minter and the rank it publishes agree.
///
/// Candidates are the peers we can *see*: the gossip overlay's online
/// set plus ourselves. Scoping to online members is what keeps this
/// live — an offline lowest-ranked member must not veto recovery for
/// everyone else. The cost is that two peers with different online
/// views can both elect themselves. That degrades to a resolvable tie
/// rather than a fork: both mint at the same generation, both stamp
/// their election rank, and `convergence::incoming_wins_same_generation`
/// converges every peer on the lower-ranked key.
fn elected_to_mint(state: &Arc<AppState>, community_id: &str) -> bool {
    let Some(me) = super::mek_rotation_support::my_pseudonym(state, community_id) else {
        return false;
    };
    let mut candidates = vec![me.clone()];
    {
        let communities = state.communities.read();
        if let Some(gossip) = communities
            .get(community_id)
            .and_then(|c| c.gossip.as_ref())
        {
            for pseudonym in gossip.online_members.keys() {
                let key = PseudonymKey::from_hex_lossy(pseudonym);
                if key != me {
                    candidates.push(key);
                }
            }
        }
    }
    select_rotator(&PseudonymKey([0u8; 32]), &candidates) == Some(me)
}

/// Community-MEK recovery after vault loss (architecture §7.3): re-acquire the
/// canonical key from an online peer via `RequestMEK`; only if no peer serves
/// it within the cascade window AND [`elected_to_mint`] picks us, mint a
/// SUPERSEDING key (`rotate_mek_local` mints at `mek_generation + 1` and
/// broadcasts), so a peer that appears later converges FORWARD via
/// Max-Register instead of forking. Never mints a colliding same-generation
/// key.
pub fn spawn_community_mek_recovery(
    app_handle: tauri::AppHandle,
    state: Arc<AppState>,
    community_id: String,
    requester_pseudonym: String,
) {
    tokio::spawn(async move {
        // Snapshot before requesting so we can tell whether anything landed.
        let initial =
            crate::state_helpers::channel_media_mek(&state, &community_id, "").map(|(_, g)| g);

        // Prefer re-acquisition: ask the deterministic responder for the
        // current community MEK (empty channel + generation 0 = "send current").
        spawn_mek_request_with_retry(
            Arc::clone(&state),
            community_id.clone(),
            String::new(),
            0,
            requester_pseudonym,
        );

        // Wait out the full cascade budget (lock-step with the requester's
        // MAX_CASCADES × 5s) plus slack for the reply to apply.
        let max_cascades = u64::try_from(rekindle_mek_rotation::MAX_CASCADES).unwrap_or(3);
        let window = std::time::Duration::from_millis(max_cascades * 5_000 + 3_000);
        tokio::time::sleep(window).await;

        let current =
            crate::state_helpers::channel_media_mek(&state, &community_id, "").map(|(_, g)| g);
        if !should_last_resort_mint(initial, current, elected_to_mint(&state, &community_id)) {
            return;
        }

        tracing::warn!(
            community = %community_id,
            "no peer served the community MEK within the re-acquire window — \
             minting a superseding key (last resort)"
        );
        if let Err(e) = crate::services::community_mek_local_rotate::rotate_mek_local(
            &app_handle,
            &state,
            &community_id,
        )
        .await
        {
            tracing::warn!(
                community = %community_id,
                error = %e,
                "last-resort community MEK mint failed"
            );
        }
    });
}

/// Phase 23.D.10 — facade around `rekindle_mek_rotation::handle_incoming_mek_transfer`.
/// Constructs a `MekAdapter` per call and delegates.
pub fn handle_incoming_mek_transfer(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: Option<&str>,
    sender_pseudonym: &str,
    wrapped_mek: &[u8],
) -> Result<u64, String> {
    let pool = tauri::Manager::try_state::<crate::db::DbPool>(app_handle)
        .ok_or_else(|| "DbPool state missing".to_string())?
        .inner()
        .clone();
    let adapter =
        crate::services::mek_adapter::MekAdapter::new(Arc::clone(state), app_handle.clone(), pool);
    rekindle_mek_rotation::handle_incoming_mek_transfer(
        adapter.as_ref(),
        community_id,
        channel_id,
        sender_pseudonym,
        wrapped_mek,
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::should_last_resort_mint;

    #[test]
    fn elected_mints_only_when_nothing_landed() {
        // Elected, slot still empty after the window → mint (no peer served it).
        assert!(should_last_resort_mint(None, None, true));
    }

    #[test]
    fn peer_served_key_blocks_mint() {
        // A peer served the canonical key during the window → keep it, no mint.
        assert!(!should_last_resort_mint(None, Some(1), true));
        assert!(!should_last_resort_mint(None, Some(7), true));
    }

    #[test]
    fn unelected_never_mints() {
        assert!(!should_last_resort_mint(None, None, false));
        assert!(!should_last_resort_mint(None, Some(3), false));
    }

    #[test]
    fn nonempty_initial_never_mints() {
        // Defensive: recovery only runs on an absent slot; if we somehow had a
        // key to begin with, never mint a competing one.
        assert!(!should_last_resort_mint(Some(2), None, true));
        assert!(!should_last_resort_mint(Some(2), Some(2), true));
    }
}

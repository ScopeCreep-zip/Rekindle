//! Phase 21 REDO — central `presence_poll_tick` orchestrator.
//!
//! Pre-port lived in
//! `src-tauri/services/community/presence/poll.rs` as a 330-LoC
//! state-bound function. Here it's parameterised over
//! [`CommunityPresenceDeps`] so the entire pipeline — registry
//! open, presence write, per-segment Plate Gate scan, role/profile/
//! RSVP merge, stale-sync
//! retry, auto-expand — is observable + testable.
//!
//! Architecture references:
//! - §13.4 presence cadence
//! - §14.3 Shared Locker history advertisement
//! - §14.5 mutual aid + peer reliability
//! - §15 Plate Gate per-segment scan + auto-expand
//! - §26 W26 — presence row signature verification

use std::sync::Arc;
use std::time::Duration;

use crate::deps::CommunityPresenceDeps;

/// 180-second heartbeat freshness window. Presence rows with a
/// last_heartbeat older than `now - 180s` are treated as offline
/// during the segment scan.
pub const STALE_HEARTBEAT_SECS: u64 = 180;

/// Compute the gossip fan-out degree for an online population.
/// Mirrors src-tauri's `state::gossip_degree` and architecture §3.
#[must_use]
pub fn gossip_degree(n: usize) -> usize {
    match n {
        0 => 0,
        1..=20 => n.min(6),
        21..=60 => 6,
        _ => 8,
    }
}

/// Run one presence-poll tick for the community.
///
/// Returns `Err(reason)` when:
/// - we're not attached to Veilid (`"not attached"`)
/// - the community isn't joined (`"community not found"`)
/// - the member registry key is unset (the orchestrator logs +
///   returns `Ok` so the cadence loop keeps polling).
pub async fn presence_poll_tick<D: CommunityPresenceDeps>(
    deps: Arc<D>,
    community_id: &str,
) -> Result<(), String> {
    run_tick(deps, community_id, Tick::Beat).await
}

/// A watched registry (or Plate Gate segment) changed: read the rows and
/// merge them, as a tick does, but never write our own row (D4; the beat
/// is the cadence's, 60 s, `spawn.rs`). A write here was the Linux storm
/// of round 7 (finding IB1c): each member's row write fired the other's
/// registry watch, whose tick wrote again, ~45 writes a minute on both
/// sides once the joiner watched the registry too (IB1's hand-over).
///
/// # Errors
/// As [`presence_poll_tick`].
pub async fn presence_refresh<D: CommunityPresenceDeps>(
    deps: Arc<D>,
    community_id: &str,
) -> Result<(), String> {
    run_tick(deps, community_id, Tick::Refresh).await
}

/// What a tick does with our own row.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tick {
    /// The cadence's beat: write our row, then scan.
    Beat,
    /// A watch's change: scan only.
    Refresh,
}

async fn run_tick<D: CommunityPresenceDeps>(
    deps: Arc<D>,
    community_id: &str,
    tick: Tick,
) -> Result<(), String> {
    let Some(registry_key) = deps.ensure_registry_open(community_id).await? else {
        tracing::warn!(
            community = %community_id,
            "presence_poll_tick: member_registry_key is None — skipping (join may be pending)",
        );
        return Ok(());
    };

    let Some(creds) = deps.presence_credentials(community_id) else {
        return Err("community not found".to_string());
    };

    // A beat computes the history ranges (Shared Locker §14.3) then
    // writes our own presence row to the registry; a refresh only reads.
    if tick == Tick::Beat {
        let history_ranges = deps.compute_history_ranges(community_id).await;
        if deps.scope().is_closed() {
            return Ok(());
        }
        crate::community::registry::write_our_presence(
            deps.as_ref(),
            crate::community::registry::PresenceWrite {
                community_id,
                registry_key: &registry_key,
                my_pseudonym_hex: &creds.my_pseudonym_hex,
                my_subkey_index: creds.my_subkey_index,
                slot_keypair_str: creds.slot_keypair_str.as_deref(),
                has_slot_seed: creds.slot_seed_hex.is_some(),
                history_ranges,
            },
        )
        .await;
    }

    // Per-segment Plate Gate scan (architecture §15.5). Each segment
    // is its own SMPL record carrying ≤255 LOCAL subkeys; we skip
    // our own subkey on our own segment.
    let banned = deps.governance_bans(community_id);
    let now_secs = deps.now_secs();
    let descriptors = deps.segment_descriptors(community_id);
    let my_subkey = creds.my_subkey_index.unwrap_or(u32::MAX);

    // Read-side reciprocity policy (local-only, default-deny): a signal
    // we don't share, we don't get to read off peers' rows. Read once
    // before the scan so every row is filtered against the same policy.
    let our_policy = deps.presence_policy(community_id);

    let mut discovered = Vec::new();
    let mut online_count = 0usize;
    let mut known_member_keys = std::collections::HashSet::new();
    let mut voice_rows: Vec<crate::deps::VoicePresenceRow> = Vec::new();
    for descriptor in &descriptors {
        if deps.scope().is_closed() {
            return Ok(());
        }
        let skip_subkey = (descriptor.segment_index == creds.my_segment_index).then_some(my_subkey);
        let raw_rows = deps
            .scan_segment_raw(
                &descriptor.registry_key,
                crate::community::scan_row::SUBKEYS_PER_SEGMENT,
                skip_subkey,
            )
            .await;
        tracing::debug!(
            community = %community_id,
            segment = descriptor.segment_index,
            registry_key = %descriptor.registry_key,
            rows = raw_rows.len(),
            "presence scan: segment rows read",
        );
        for (subkey, raw_bytes) in raw_rows {
            let classified = crate::community::scan_row::parse_and_classify_row(
                &raw_bytes,
                &banned,
                STALE_HEARTBEAT_SECS,
                now_secs,
            );
            log_scanned_row(
                community_id,
                descriptor.segment_index,
                subkey,
                &classified,
                now_secs,
            );
            if let crate::community::scan_row::ClassifiedRow::Accepted(row) = classified {
                let mut row = *row;
                // Read-side MEK gate: the writer moved the identity-
                // revealing signals (location/activity) out of the
                // plaintext session into a MEK-encrypted blob. Decrypt
                // with our current MEK (None when we lack the matching
                // generation — the peer just shows no location), merge
                // them back, then drop any signal our own policy doesn't
                // reciprocate so the model self-balances.
                // Voice-channel membership is cleartext on the row — read
                // it directly so discovery never depends on decrypting
                // the peer's extras (which fails during a MEK split-brain
                // and used to stall voice join for minutes).
                let row_voice_channel = row.presence.voice_channel_id.clone();
                if let Some(enc) = &row.presence.session_extras_encrypted {
                    if let Some(extras) = deps.decrypt_session_extras(community_id, enc) {
                        row.presence.session.location = extras.location;
                        row.presence.session.activity = extras.activity;
                    }
                }
                row.presence.session =
                    crate::community::filter_incoming(row.presence.session, &our_policy);
                known_member_keys.insert(row.pseudonym_hex.clone());
                // Presence-derived voice membership for the roster
                // reconcile — stale rows included so ghosts expire.
                voice_rows.push(crate::deps::VoicePresenceRow::claim(
                    row.pseudonym_hex.clone(),
                    row_voice_channel,
                    row.online_member.is_some(),
                ));
                online_count += usize::from(row.online_member.is_some());
                discovered.push((descriptor.segment_index, subkey, row.presence));
            }
        }
    }

    // Update in-memory state: known members, persist discovered rows +
    // ban deletions, event RSVP aggregation, member profile diff +
    // MembersRefreshed emit. Member roles are not the scan's: every
    // reader derives them from the merged governance's role assignments
    // (plan E3.2, D15; review RV7 M1).
    deps.apply_member_state_update(community_id, known_member_keys, &banned);
    crate::community::persist_discovered_registry_members(
        deps.as_ref(),
        community_id,
        &discovered,
        &banned,
    );
    // Voice roster backstop (three-path Path 1, MatrixRTC pattern):
    // hand the scan's presence-derived membership to the voice layer —
    // members whose VoiceJoin gossip was lost get added, ghosts whose
    // rows say "left" or whose heartbeat went stale get expired.
    deps.session_presence()
        .reconcile_voice_roster(community_id, voice_rows);
    // Member profile diff: read prior snapshots, compose via the
    // pure `compute_profile_diff`, apply + fire MembersRefreshed
    // only when at least one entry changed (wave 5 D1).
    let prior_profiles = deps.read_member_profile_snapshot(community_id);
    let profile_outcome =
        crate::community::profile_diff::compute_profile_diff(&prior_profiles, &discovered);
    deps.apply_member_profile_updates(
        community_id,
        profile_outcome.updates,
        profile_outcome.changed,
    );

    // The gossip mesh is not the scan's: the Host builds it from the
    // roster's live beats and their `M`/6 routes (plan E3.4 S5,
    // `host/adapters/presence/mesh.rs`), and a member whose beats stop
    // turns offline by the beat view's expiry, never by this poll.
    tracing::info!(
        community = %community_id,
        online_rows = online_count,
        "presence_poll_tick: registry rows merged",
    );

    // A5/P4.3 — admin-side Plate Gate auto-expand trigger
    // (architecture §15.1). Spawned by the adapter; the
    // orchestrator just fires the trigger and continues.
    deps.maybe_auto_expand_segment(community_id);

    Ok(())
}

/// Public-test entry point — pre-port `presence_poll_tick_public`
/// in src-tauri. Identical to `presence_poll_tick`; named here so
/// the existing facade call site keeps compiling.
#[inline]
pub async fn presence_poll_tick_public<D: CommunityPresenceDeps>(
    deps: Arc<D>,
    community_id: &str,
) -> Result<(), String> {
    presence_poll_tick(deps, community_id).await
}

/// Shorter helper for callers that want the polling cadence on a
/// hard-coded interval (matches the pre-port `Duration::from_secs(60)`).
#[must_use]
pub fn steady_poll_duration() -> Duration {
    Duration::from_secs(crate::community::spawn::STEADY_TICK_INTERVAL_SECS)
}

/// One scanned registry row, as the scan judged it: what decides whether a
/// member shows up and whether it can be reached.
fn log_scanned_row(
    community_id: &str,
    segment: u32,
    subkey: u32,
    classified: &crate::community::scan_row::ClassifiedRow,
    now_secs: u64,
) {
    match classified {
        crate::community::scan_row::ClassifiedRow::Accepted(row) => tracing::debug!(
            community = %community_id,
            segment,
            subkey,
            member = %row.pseudonym_hex,
            status = %row.presence.status,
            heartbeat_age_secs = now_secs.saturating_sub(row.presence.last_heartbeat),
            online = row.online_member.is_some(),
            route_len = row.presence.route_blob.len(),
            "presence scan: row accepted",
        ),
        other => tracing::debug!(
            community = %community_id,
            segment,
            subkey,
            outcome = other.label(),
            "presence scan: row not accepted",
        ),
    }
}

#[cfg(test)]
#[path = "poll_tests.rs"]
mod tests;

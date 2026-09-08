//! Phase 18.g — join-stage primitives.
//!
//! Ported from `src-tauri/src/services/community/join/flow.rs`. Heavy
//! crate-side helpers consumed by src-tauri's `join_community`
//! orchestrator (chiral split — see `join.rs` module docs).
//!
//! Hosts the multi-segment governance snapshot loader, the slot-claim
//! state machine, and the Plate Gate auto-expand-and-retry path.

use rekindle_governance::invite_quota;
use rekindle_governance::merge;
use rekindle_governance::permissions::compute_permissions;
use rekindle_governance::state::{GovernanceState, SegmentState};
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_secrets::derive;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;
use rekindle_types::permissions::MANAGE_COMMUNITY;
use rekindle_types::presence::MemberPresence;

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::event::GovernanceRuntimeEvent;
use crate::join::{default_community_name, merge_presence_entry, InitialPresence};
use crate::segments;

/// Snapshot of the multi-segment governance record set, ready for the
/// adapter to build a fresh `CommunityState`.
pub struct GovernanceSnapshot {
    pub all_entries: Vec<(PseudonymKey, Vec<GovernanceEntry>)>,
    pub gov_state: GovernanceState,
    pub name: String,
    pub description: Option<String>,
    /// Every GovernanceOverflow record key followed while loading this
    /// snapshot (primary + all segments). The src-tauri join flow seeds these
    /// into the new `CommunityState.open_community_records.governance_overflow_keys`
    /// so they are opened+tracked (§10) and warmed/rehydrated (§14.1) like every
    /// other community record — the joiner can't register via `community_id`
    /// here because the community isn't in `AppState` yet.
    pub overflow_keys: Vec<String>,
}

/// Shared join-cursor state threaded through the slot-claim state
/// machine. All three claim functions (`claim_registry_slot`,
/// `try_claim_in_candidates`, `auto_expand_and_retry`) borrow the same
/// cluster of fields; bundling them here keeps each signature small and
/// the cursor immutable for the duration of a join attempt.
///
/// The generic `D: GovernanceRuntimeDeps` stays a function type param
/// (not on the struct) so the adapter type never leaks into the cursor.
/// Architecture §6.2 Step 9 — how many contended subkeys a joiner will
/// step over within one segment before giving up on it. Five, per the
/// spec; each attempt costs one CAS write plus one verifying read.
const MAX_SLOT_CLAIM_ATTEMPTS: u32 = 5;

pub struct SlotClaimCtx<'a> {
    /// Community identifier (used for segment expansion + mesh control).
    pub community_id: &'a str,
    /// Segment-0 invite registry key — the inviter's registry.
    pub invite_registry_key: &'a str,
    /// Inviter's pseudonym — drives the M10.3 invite-quota check. `None` when
    /// the invite was accepted from its link pointer and governance carried no
    /// matching `InviteCreated` to attribute it to an inviter; the best-effort
    /// quota cap is then skipped (the inviter still enforces it at write time).
    pub inviter_pseudonym: Option<&'a PseudonymKey>,
    /// Joiner's own pseudonym key.
    pub my_pseudo: &'a PseudonymKey,
    /// Joiner's pseudonym signing key for presence + slot writes.
    pub pseudonym_signing: &'a SigningKey,
    /// Merged governance state at the start of the join attempt.
    pub gov_state: &'a GovernanceState,
    /// Presence `status` label written into the claimed slot.
    pub join_status_label: &'a str,
    /// Optional display name written into the claimed slot.
    pub display_name: Option<String>,
}

/// Outcome of a successful slot claim — segment + local subkey within
/// that segment + the (string-formatted) writer keypair used for future
/// writes. The adapter persists this into `CommunityState`.
pub struct ClaimedSlot {
    pub registry_key: String,
    pub segment_index: u32,
    pub local_subkey: u32,
    pub slot_keypair_str: String,
    /// Registry subkeys that held a value at claim time — the present-set from
    /// the ClaimSlot inspect (architecture §6.2 Step 7). `collect_initial_presence_state`
    /// (Step 12) reads ONLY these, never a blind `0..255` sweep: each empty
    /// subkey's cold `get_dht_value` blocks up to `get_value_timeout_ms` (10 s).
    pub occupied_subkeys: Vec<u32>,
    /// The joiner's own signed `MemberPresence` row, read back + verified from
    /// the slot it just claimed. The src-tauri orchestrator persists it into
    /// `community_members` so the joiner appears in their own roster
    /// immediately — mirroring the creator self-row persist in
    /// `origin::create_community` (Members panel populates without waiting for
    /// the steady presence poll).
    pub self_presence: MemberPresence,
}

/// First-pass + multi-segment governance snapshot: fetch + W26-verify
/// every signed `GovernanceSubkeyPayload` from the primary record, merge
/// to find announced segments, fetch + verify each segment, re-merge.
pub async fn load_governance_snapshot<D: GovernanceRuntimeDeps>(
    deps: &D,
    governance_key_str: &str,
) -> Result<GovernanceSnapshot, GovernanceRuntimeError> {
    let primary = fetch_governance_record_entries(deps, governance_key_str).await?;
    let mut all_entries = primary.authored;
    let mut overflow_keys = primary.overflow_keys;
    let gov_state_v1 = merge::merge(&all_entries);

    // Pass 2: fetch every segment's governance record. CRDT idempotence +
    // commutativity (Almeida 2016 §3) means re-merge is canonical
    // regardless of fetch order.
    for segment in &gov_state_v1.segments {
        if segment.segment_index == 0 {
            continue;
        }
        match fetch_governance_record_entries(deps, &segment.governance_key).await {
            Ok(mut extra) => {
                all_entries.append(&mut extra.authored);
                overflow_keys.append(&mut extra.overflow_keys);
            }
            Err(e) => tracing::warn!(
                segment = segment.segment_index,
                governance_key = %segment.governance_key,
                error = %e,
                "load_governance_snapshot: skipping unreachable segment governance record"
            ),
        }
    }
    let gov_state = if gov_state_v1.segments.iter().any(|s| s.segment_index > 0) {
        merge::merge(&all_entries)
    } else {
        gov_state_v1
    };

    let name = gov_state.metadata.as_ref().map_or_else(
        || default_community_name(governance_key_str),
        |metadata| metadata.name.clone(),
    );
    let description = gov_state
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.description.clone());

    overflow_keys.sort();
    overflow_keys.dedup();

    Ok(GovernanceSnapshot {
        all_entries,
        gov_state,
        name,
        description,
        overflow_keys,
    })
}

/// Subkeys an `UpdateGet` inspect reports as holding a value, or the full
/// `0..255` range when the inspect call fails.
///
/// A blind `0..255` `get_dht_value` sweep costs 255 serial network
/// round-trips on a cold join — the local store is empty until each subkey
/// is fetched, and ~250 of those land on empty slots, enough to exceed the
/// UI's join timeout. One inspect collapses that to a single network
/// round-trip plus a fetch per populated subkey. On inspect error we fall
/// back to the full sweep so a join never silently drops entries.
async fn populated_subkeys_or_full_scan<D: GovernanceRuntimeDeps>(
    deps: &D,
    record_key: &str,
) -> Vec<u32> {
    match deps.inspect_dht_record_present_subkeys(record_key).await {
        Ok(subkeys) => subkeys,
        Err(e) => {
            tracing::warn!(
                record = %record_key,
                error = %e,
                "inspect failed; falling back to full 0..255 subkey scan"
            );
            (0..255u32).collect()
        }
    }
}

/// Read the populated subkeys of a single governance record, W26-verify each
/// payload's pseudonym signature (any member could write to any slot, so the
/// signature is the only authorship proof), and follow every author's
/// `overflow_next` chain so a spilled author's log is reassembled in full
/// before merge (architecture §"Follow GovernanceOverflow pointers", line 1609).
async fn fetch_governance_record_entries<D: GovernanceRuntimeDeps>(
    deps: &D,
    governance_key_str: &str,
) -> Result<crate::overflow::GovernanceReadout, GovernanceRuntimeError> {
    deps.open_dht_record(governance_key_str, None).await?;
    let occupied = populated_subkeys_or_full_scan(deps, governance_key_str).await;
    Ok(crate::overflow::read_governance_with_overflow(deps, governance_key_str, &occupied).await)
}

/// (segment_index, registry_key, slot_range_start) for each segment to
/// try in claim order. Segment 0 is the inviter's registry; later
/// segments come from merged governance's `SegmentAdded` entries.
struct SegmentClaimCandidate {
    segment_index: u32,
    registry_key: String,
    slot_range_start: u32,
}

/// Inner outcome of one slot-claim attempt sweep over the candidate list.
struct ClaimAttemptOutcome {
    claimed: Option<ClaimedSlot>,
    last_full_segment: Option<u32>,
}

/// Claim a registry slot for the joiner. Tries segment 0 first, then
/// every additional segment merged from governance state. If all
/// segments are full, delegates to `auto_expand_and_retry` (P4.3 Plate
/// Gate).
///
/// M10.3 — joiner-side invite quota check runs before any slot write as
/// defense-in-depth (reader-validates also drops over-quota
/// `InviteCreated` entries at merge time).
pub async fn claim_registry_slot<D: GovernanceRuntimeDeps>(
    deps: &D,
    slot_seed_hex: &str,
    ctx: SlotClaimCtx<'_>,
) -> Result<ClaimedSlot, GovernanceRuntimeError> {
    if let Some(inviter) = ctx.inviter_pseudonym {
        if !invite_quota::check_active_invites_cap(ctx.gov_state, inviter) {
            return Err(GovernanceRuntimeError::Adapter(
                "invite quota exceeded for inviter — community is rate-limiting joins".into(),
            ));
        }
    }

    let slot_seed_bytes: [u8; 32] = hex::decode(slot_seed_hex)
        .map_err(|e| GovernanceRuntimeError::Crypto(format!("invalid slot seed hex: {e}")))?
        .try_into()
        .map_err(|_| GovernanceRuntimeError::Crypto("slot seed must be 32 bytes".into()))?;

    let outcome =
        try_claim_in_candidates(deps, &ctx, &slot_seed_bytes, &ctx.gov_state.segments).await?;
    if let Some(claimed) = outcome.claimed {
        return Ok(claimed);
    }

    if let Some(full_seg) = outcome.last_full_segment {
        return auto_expand_and_retry(deps, &ctx, &slot_seed_bytes, full_seg).await;
    }
    Err(GovernanceRuntimeError::Adapter(
        "No reachable segment registry — Veilid attach may have failed".into(),
    ))
}

async fn try_claim_in_candidates<D: GovernanceRuntimeDeps>(
    deps: &D,
    ctx: &SlotClaimCtx<'_>,
    slot_seed_bytes: &[u8; 32],
    governance_segments: &[SegmentState],
) -> Result<ClaimAttemptOutcome, GovernanceRuntimeError> {
    let mut candidates: Vec<SegmentClaimCandidate> = Vec::new();
    candidates.push(SegmentClaimCandidate {
        segment_index: 0,
        registry_key: ctx.invite_registry_key.to_string(),
        slot_range_start: 0,
    });
    for seg in governance_segments {
        if seg.segment_index == 0 {
            continue;
        }
        candidates.push(SegmentClaimCandidate {
            segment_index: seg.segment_index,
            registry_key: seg.registry_key.clone(),
            slot_range_start: seg.slot_range_start,
        });
    }
    candidates.sort_by_key(|c| c.segment_index);

    let mut last_full_segment: Option<u32> = None;
    for candidate in &candidates {
        if let Err(e) = deps.open_dht_record(&candidate.registry_key, None).await {
            tracing::warn!(
                segment = candidate.segment_index,
                error = %e,
                "claim_registry_slot: failed to open segment registry — skipping"
            );
            continue;
        }
        // Architecture §6.2 Step 7: one inspect fanout → the present-subkey set.
        // Empty = "no value present" (not "seq == 0") so an occupied-but-seq-0
        // slot is never mis-claimed; the same set is reused by Step 12 presence
        // collection so it reads only occupied slots, never a blind 0..255 sweep.
        let present = deps
            .inspect_dht_record_present_subkeys(&candidate.registry_key)
            .await?;
        let mut present_set: std::collections::HashSet<u32> = present.iter().copied().collect();

        // Architecture §6.2 Step 9: "on conflict, retry next slot
        // (max 5)". A fixed 255-subkey array makes two joiners picking
        // "the lowest free subkey" contend by design, so a contended
        // subkey joins the occupied set and the next attempt skips it
        // rather than re-racing the same index. `reclaim` documents why
        // the array is fixed at all.
        let mut claimed_slot = None;
        // Checked at most once per segment and only when it looks full,
        // so an ordinary join still costs one inspect and no row fetches.
        let mut reclaim_checked = false;
        for _attempt in 0..MAX_SLOT_CLAIM_ATTEMPTS {
            // Ascending `find` is MLS RFC 9420 §7.1's leftmost-blank rule.
            let mut pick = (0..255u32).find(|subkey| !present_set.contains(subkey));
            if pick.is_none() && !reclaim_checked {
                reclaim_checked = true;
                let reusable = reclaim::reclaimable_slots(
                    deps,
                    &candidate.registry_key,
                    &present,
                    ctx.gov_state,
                )
                .await;
                tracing::debug!(
                    segment = candidate.segment_index,
                    reusable = reusable.len(),
                    "segment full; checked for non-member slots"
                );
                for subkey in reusable {
                    present_set.remove(&subkey);
                }
                pick = (0..255u32).find(|subkey| !present_set.contains(subkey));
            }
            let Some(local_subkey) = pick else {
                last_full_segment = Some(candidate.segment_index);
                break;
            };

            let global_slot = candidate.slot_range_start + local_subkey;
            let slot_kp =
                derive::derive_slot_keypair(slot_seed_bytes, global_slot).map_err(|e| {
                    GovernanceRuntimeError::Crypto(format!("slot keypair derivation failed: {e}"))
                })?;
            let slot_kp_str =
                deps.format_writer_keypair(slot_kp.verifying_key().to_bytes(), slot_kp.to_bytes());

            let mut presence = MemberPresence {
                pseudonym_key: ctx.my_pseudo.clone(),
                display_name: ctx.display_name.clone(),
                status: ctx.join_status_label.into(),
                route_blob: vec![],
                last_heartbeat: rekindle_utils::timestamp_secs(),
                ..Default::default()
            };
            let presence_sig =
                derive::sign_with_pseudonym(ctx.pseudonym_signing, &presence.signing_bytes());
            presence.signature = presence_sig.to_vec();
            let presence_bytes = serde_json::to_vec(&presence).map_err(|e| {
                GovernanceRuntimeError::Encoding(format!("presence serialization failed: {e}"))
            })?;

            // The compare-and-swap. `Some(_)` means the network already
            // held a newer value for this subkey — someone beat us to it.
            // This branch was unreachable until `record::set` stopped
            // discarding veilid's return value, which is what made two
            // joiners able to both believe they had claimed one slot.
            let write_outcome = deps
                .set_dht_value(
                    &candidate.registry_key,
                    local_subkey,
                    presence_bytes,
                    Some(slot_kp_str.clone()),
                )
                .await?;
            if write_outcome.is_some() {
                tracing::debug!(
                    segment = candidate.segment_index,
                    local_subkey,
                    "slot claim lost the race — trying the next free subkey"
                );
                present_set.insert(local_subkey);
                continue;
            }

            let verify_bytes = deps
                .get_dht_value(&candidate.registry_key, local_subkey, true)
                .await?
                .ok_or(GovernanceRuntimeError::VerifyEmpty)?;
            let written: MemberPresence = serde_json::from_slice(&verify_bytes).map_err(|e| {
                GovernanceRuntimeError::Encoding(format!(
                    "slot read-back deserialization failed: {e}"
                ))
            })?;
            // Read-back mismatch is the same race seen a moment later:
            // our write landed but was overwritten before we re-read.
            // Treat it exactly like a CAS failure rather than aborting.
            if written.pseudonym_key != *ctx.my_pseudo {
                tracing::debug!(
                    segment = candidate.segment_index,
                    local_subkey,
                    "slot read-back shows another member — trying the next free subkey"
                );
                present_set.insert(local_subkey);
                continue;
            }

            claimed_slot = Some(ClaimedSlot {
                registry_key: candidate.registry_key.clone(),
                segment_index: candidate.segment_index,
                local_subkey,
                slot_keypair_str: slot_kp_str,
                occupied_subkeys: present_set.iter().copied().collect(),
                self_presence: written,
            });
            break;
        }

        if let Some(claimed) = claimed_slot {
            return Ok(ClaimAttemptOutcome {
                claimed: Some(claimed),
                last_full_segment,
            });
        }
    }

    Ok(ClaimAttemptOutcome {
        claimed: None,
        last_full_segment,
    })
}

async fn auto_expand_and_retry<D: GovernanceRuntimeDeps>(
    deps: &D,
    ctx: &SlotClaimCtx<'_>,
    slot_seed_bytes: &[u8; 32],
    full_segment_index: u32,
) -> Result<ClaimedSlot, GovernanceRuntimeError> {
    let community_id = ctx.community_id;
    let perms = compute_permissions(
        ctx.my_pseudo,
        None,
        ctx.gov_state,
        rekindle_utils::timestamp_secs(),
    );
    let have_manage_community = (perms & MANAGE_COMMUNITY) != 0;
    let requester_pseudonym = hex::encode(ctx.my_pseudo.0);

    deps.emit_event(GovernanceRuntimeEvent::JoinPendingAlert {
        have_manage_community,
    });

    if have_manage_community {
        tracing::info!(
            community = %community_id,
            full_segment_index,
            "Plate Gate: joiner has MANAGE_COMMUNITY — expanding inline"
        );
        segments::expand_community_segment(deps, community_id)
            .await
            .map_err(|e| {
                GovernanceRuntimeError::Adapter(format!("inline segment expansion failed: {e}"))
            })?;
    } else {
        tracing::info!(
            community = %community_id,
            full_segment_index,
            "Plate Gate: joiner lacks MANAGE_COMMUNITY — gossiping RequestSegmentExpansion"
        );
        let envelope = CommunityEnvelope::Control(ControlPayload::RequestSegmentExpansion {
            community_id: community_id.to_string(),
            requester_pseudonym,
            full_segment_index,
        });
        deps.send_to_mesh(community_id, &envelope)?;
    }

    // Poll up to 30s for the new segment to land in our merged
    // governance state. Quicker than a watch-driven notify, simpler than
    // a Notify channel, and bounded.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut new_segments: Vec<SegmentState>;
    loop {
        new_segments = deps
            .governance_state(community_id)
            .map(|gov| gov.segments)
            .unwrap_or_default();
        let max_seg = new_segments
            .iter()
            .map(|s| s.segment_index)
            .max()
            .unwrap_or(0);
        if max_seg > full_segment_index {
            break;
        }
        if std::time::Instant::now() >= deadline {
            return Err(GovernanceRuntimeError::Adapter(format!(
                "Community is full and no admin expanded within 30s. The community has {} active segment(s); admins must run expand_community_segment to grow it.",
                new_segments.len() + 1
            )));
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    let outcome = try_claim_in_candidates(deps, ctx, slot_seed_bytes, &new_segments).await?;
    if let Some(claimed) = outcome.claimed {
        return Ok(claimed);
    }
    Err(GovernanceRuntimeError::Adapter(
        "Community segment expanded but still full — concurrent joiner race; please retry".into(),
    ))
}

/// Architecture §6.2 Step 12 — read + W26-verify the `MemberPresence` row in
/// each occupied registry slot (the `occupied_subkeys` set captured by the
/// Step 7 ClaimSlot inspect), populating the initial `peers` +
/// `online_members` + `known_members` sets the adapter plumbs into
/// `GossipOverlay`. Reading only occupied slots avoids the cold-join 10 s
/// `get_value_timeout_ms` stall a blind `0..255` sweep incurs on empty slots.
pub async fn collect_initial_presence_state<D: GovernanceRuntimeDeps>(
    deps: &D,
    registry_key: &str,
    my_slot: u32,
    my_pseudo_hex: &str,
    occupied_subkeys: &[u32],
) -> InitialPresence {
    use futures::stream::{FuturesUnordered, StreamExt};

    let started = std::time::Instant::now();
    let mut presence = InitialPresence::default();
    presence.known_members.insert(my_pseudo_hex.to_string());

    // Architecture §6.2 Step 12: read `MemberPresence` ONLY from the slots the
    // Step 7 (ClaimSlot) inspect reported occupied. A blind `0..255` sweep is
    // fatal on a cold join — every empty subkey's
    // `get_dht_value(force_refresh = false)` blocks up to `get_value_timeout_ms`
    // (10 s) on a network RPC, busting the 10 s CollectPresence gate. The +10 s
    // steady-state presence poll (`flow.rs`) and the DHT value watches backfill
    // anyone who claimed a slot after our Step 7 inspect.
    const SCAN_PARALLELISM: usize = 16;
    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(SCAN_PARALLELISM));
    let mut scans = FuturesUnordered::new();
    for &subkey in occupied_subkeys {
        if subkey == my_slot {
            continue;
        }
        let sem = std::sync::Arc::clone(&sem);
        scans.push(async move {
            let _permit = sem.acquire().await.expect("scan semaphore not closed");
            (
                subkey,
                deps.get_dht_value(registry_key, subkey, false).await,
            )
        });
    }

    let mut verified = 0usize;
    while let Some((subkey, result)) = scans.next().await {
        let Ok(Some(bytes)) = result else {
            continue;
        };
        if bytes.is_empty() {
            continue;
        }
        let Ok(row) = serde_json::from_slice::<MemberPresence>(&bytes) else {
            continue;
        };
        let Ok(sig_arr): Result<[u8; 64], _> = row.signature.as_slice().try_into() else {
            continue;
        };
        if derive::verify_pseudonym_signature(&row.pseudonym_key.0, &row.signing_bytes(), &sig_arr)
            .is_err()
        {
            continue;
        }
        let pseudo_hex = hex::encode(row.pseudonym_key.0);
        merge_presence_entry(
            &mut presence,
            &pseudo_hex,
            &row.status,
            &row.route_blob,
            row.last_heartbeat,
        );
        // Keep the full verified row for the durable roster persist in the
        // src-tauri orchestrator (`community_members`). `merge_presence_entry`
        // only borrowed the liveness/route fields above; the whole row carries
        // display_name/roles/profile the roster needs.
        presence.discovered.push((subkey, row));
        verified += 1;
    }

    tracing::info!(
        registry = %registry_key,
        occupied = occupied_subkeys.len(),
        verified,
        elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        "collect_initial_presence_state: cold-join presence scan complete"
    );
    presence
}

mod reclaim;

#[cfg(test)]
mod tests;

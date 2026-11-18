//! Phase 18.g — join-stage primitives.
//!
//! Ported from `src-tauri/src/services/community/join/flow.rs`. Heavy
//! crate-side helpers consumed by src-tauri's `join_community`
//! orchestrator (chiral split — see `join.rs` module docs).
//!
//! Hosts the slot-claim state machine and the cold-join presence scan.

use rekindle_governance::invite_quota;
use rekindle_governance::state::GovernanceState;
use rekindle_records::port::OpenIntent;
use rekindle_secrets::derive;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::id::PseudonymKey;
use rekindle_types::presence::MemberPresence;

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::join::{merge_presence_entry, InitialPresence};

/// Architecture §6.2 Step 9 — how many contended subkeys a joiner will
/// step over in the registry before giving up. Five, per the
/// spec; each attempt costs one CAS write plus one verifying read.
const MAX_SLOT_CLAIM_ATTEMPTS: u32 = 5;

/// Shared join-cursor state threaded through the slot claim; bundling it
/// keeps each signature small and the cursor immutable for the duration of
/// a join attempt. The generic `D: GovernanceRuntimeDeps` stays a function
/// type param (not on the struct) so the adapter type never leaks into it.
pub struct SlotClaimCtx<'a> {
    /// Community identifier.
    pub community_id: &'a str,
    /// The invite's registry key — the community's slot registry.
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

/// Outcome of a successful slot claim — the registry subkey + the (string-formatted) writer keypair used for future
/// writes. The adapter persists this into `CommunityState`.
pub struct ClaimedSlot {
    pub registry_key: String,
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

/// Claim a slot in the community's registry for the joiner.
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

    let lease = deps
        .acquire_record(ctx.invite_registry_key, None, OpenIntent::Check)
        .await
        .map_err(|e| {
            GovernanceRuntimeError::Adapter(format!(
                "No reachable registry — Veilid attach may have failed: {e}"
            ))
        })?;
    let attempt = claim_in_registry(deps, &ctx, &slot_seed_bytes, lease).await;
    deps.release_record(lease).await;
    let (claimed, full) = attempt?;
    if let Some(claimed) = claimed {
        return Ok(claimed);
    }
    Err(GovernanceRuntimeError::Adapter(if full {
        "Community is full: every registry slot is held".into()
    } else {
        "Could not claim a registry slot; please retry".into()
    }))
}

/// The claim attempts on a borrow of the registry. Returns the slot it
/// claimed, and whether the registry proved full.
async fn claim_in_registry<D: GovernanceRuntimeDeps>(
    deps: &D,
    ctx: &SlotClaimCtx<'_>,
    slot_seed_bytes: &[u8; 32],
    lease: rekindle_records::lease::LeaseId,
) -> Result<(Option<ClaimedSlot>, bool), GovernanceRuntimeError> {
    let mut registry_full = false;
    // Architecture §6.2 Step 7: one inspect fanout → the present-subkey set.
    // Empty = "no value present" (not "seq == 0") so an occupied-but-seq-0
    // slot is never mis-claimed; the same set is reused by Step 12 presence
    // collection so it reads only occupied slots, never a blind 0..255 sweep.
    let present = deps.inspect_dht_record_present_subkeys(lease).await?;
    let mut present_set: std::collections::HashSet<u32> = present.iter().copied().collect();

    // Architecture §6.2 Step 9: "on conflict, retry next slot
    // (max 5)". A fixed 255-subkey array makes two joiners picking
    // "the lowest free subkey" contend by design, so a contended
    // subkey joins the occupied set and the next attempt skips it
    // rather than re-racing the same index. `reclaim` documents why
    // the array is fixed at all.
    let mut claimed_slot = None;
    // Checked at most once and only when the registry looks full,
    // so an ordinary join still costs one inspect and no row fetches.
    let mut reclaim_checked = false;
    for _attempt in 0..MAX_SLOT_CLAIM_ATTEMPTS {
        if crate::join_gate::should_stop(deps) {
            break;
        }
        // Ascending `find` is MLS RFC 9420 §7.1's leftmost-blank rule.
        let mut pick = (0..255u32).find(|subkey| !present_set.contains(subkey));
        if pick.is_none() && !reclaim_checked {
            reclaim_checked = true;
            let reusable =
                reclaim::reclaimable_slots(deps, ctx.invite_registry_key, &present, ctx.gov_state)
                    .await;
            tracing::debug!(
                reusable = reusable.len(),
                "registry full; checked for non-member slots"
            );
            for subkey in reusable {
                present_set.remove(&subkey);
            }
            pick = (0..255u32).find(|subkey| !present_set.contains(subkey));
        }
        let Some(local_subkey) = pick else {
            registry_full = true;
            break;
        };

        let slot_kp = derive::derive_slot_keypair(slot_seed_bytes, local_subkey).map_err(|e| {
            GovernanceRuntimeError::Crypto(format!("slot keypair derivation failed: {e}"))
        })?;
        let slot_kp_str =
            deps.format_writer_keypair(slot_kp.verifying_key().to_bytes(), slot_kp.to_bytes());

        let mut presence = MemberPresence {
            pseudonym_key: ctx.my_pseudo.clone(),
            display_name: ctx.display_name.clone(),
            status: ctx.join_status_label.into(),
            // The route we already publish, as every later presence write
            // carries it: the inviter answers our `RequestMEK` only once
            // its scan reads a route on this row.
            route_blob: deps.our_route_blob(),
            last_heartbeat: deps.clock().now_secs(),
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
                lease,
                local_subkey,
                presence_bytes,
                Some(slot_kp_str.clone()),
            )
            .await?;
        if write_outcome.is_some() {
            tracing::debug!(
                local_subkey,
                "slot claim lost the race — trying the next free subkey"
            );
            present_set.insert(local_subkey);
            continue;
        }

        let verify_bytes = deps
            .get_dht_value(lease, local_subkey, true)
            .await?
            .ok_or(GovernanceRuntimeError::VerifyEmpty)?;
        let written: MemberPresence = serde_json::from_slice(&verify_bytes).map_err(|e| {
            GovernanceRuntimeError::Encoding(format!("slot read-back deserialization failed: {e}"))
        })?;
        // Read-back mismatch is the same race seen a moment later:
        // our write landed but was overwritten before we re-read.
        // Treat it exactly like a CAS failure rather than aborting.
        if written.pseudonym_key != *ctx.my_pseudo {
            tracing::debug!(
                local_subkey,
                "slot read-back shows another member — trying the next free subkey"
            );
            present_set.insert(local_subkey);
            continue;
        }

        claimed_slot = Some(ClaimedSlot {
            registry_key: ctx.invite_registry_key.to_string(),
            local_subkey,
            slot_keypair_str: slot_kp_str,
            occupied_subkeys: present_set.iter().copied().collect(),
            self_presence: written,
        });
        break;
    }
    Ok((claimed_slot, registry_full))
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

    let started = tokio::time::Instant::now();
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
    // One borrow for the whole scan, released once every read is done.
    let Ok(lease) = deps
        .acquire_record(registry_key, None, OpenIntent::Check)
        .await
    else {
        return presence;
    };
    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(SCAN_PARALLELISM));
    let mut scans = FuturesUnordered::new();
    for &subkey in occupied_subkeys {
        if subkey == my_slot {
            continue;
        }
        let sem = std::sync::Arc::clone(&sem);
        scans.push(async move {
            let _permit = sem.acquire().await.expect("scan semaphore not closed");
            (subkey, deps.get_dht_value(lease, subkey, false).await)
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
    drop(scans);
    deps.release_record(lease).await;

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
pub(crate) mod registry_scan;

#[cfg(test)]
mod tests;

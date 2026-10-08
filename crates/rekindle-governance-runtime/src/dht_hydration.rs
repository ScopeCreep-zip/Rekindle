//! Phase 23.C — DHT-hydration orchestrator.
//!
//! Pre-Phase-23 the body of this lived inline in
//! `src-tauri/src/commands/auth.rs::rebuild_governance_from_dht`,
//! mixing pure logic (sig verify + CRDT merge + lamport extraction +
//! ban diff) with src-tauri concerns (Veilid IO, AppState mutation,
//! SQLite persistence). Per Invariant 7, protocol logic + state
//! mutation + CRDT merge + crypto verification are FORBIDDEN inside
//! `src-tauri/src/services/`. This module hosts the pure orchestrator;
//! the adapter implements `GovernanceRuntimeDeps` and supplies the
//! IO/state-mutation primitives.
//!
//! Call shape:
//! 1. `list_community_governance_targets()` — enumerate communities.
//! 2. For each: `open_dht_record` + `inspect_dht_record_update_get_seqs`
//!    + sequential `get_dht_value` on every occupied subkey.
//! 3. Parse each subkey payload + verify the author's pseudonym signature
//!    (architecture §26 W26). Drop forged payloads.
//! 4. CRDT merge (`rekindle_governance::merge::merge`).
//! 5. Diff `gov_state.bans` vs prior bans to compute newly observed bans.
//! 6. Extract `max_lamport` so subsequent local writes don't collide.
//! 7. `apply_governance_rebuild_result` — persist merged state +
//!    lamport restore via the adapter.
//! 8. For each new ban: `spawn_text_mek_rotation_for_ban` (fire-and-forget).

use rekindle_records::lease::{CommunityLeases, LeaseId};
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{ChannelId, PseudonymKey, RoleId};

use crate::deps::{CommunityDhtOpenSetup, GovernanceRuntimeDeps};

/// Take the session leases on governance + registry + channel-log DHT
/// records for every joined community and hand them to the host.
/// Best-effort — per-key failures are logged; the orchestrator never
/// short-circuits.
pub async fn open_community_dht_records<D: GovernanceRuntimeDeps>(deps: &D) {
    let records = deps.list_communities_for_dht_open();
    for rec in &records {
        if crate::join_gate::should_stop(deps) {
            return;
        }
        open_and_track_one_community(deps, rec).await;
    }
    tracing::info!(
        count = records.len(),
        "opened community DHT records after login"
    );
}

/// Borrow a community's channel-log and GovernanceOverflow records
/// concurrently (bounded). Best-effort — per-key failures are logged. Run
/// in parallel so a cold join's 2-3 opens overlap instead of summing. No
/// borrow starts after `until`, a deadline for starting work, never for
/// dropping it: a borrow already running finishes within the pool's own
/// budget (plan C4.L1b). Returns the leases that were taken, each with
/// its key.
async fn acquire_records_concurrent<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    keys: &[(String, Option<String>)],
    until: tokio::time::Instant,
) -> Vec<(String, LeaseId)> {
    use futures::stream::{FuturesUnordered, StreamExt};

    const OPEN_PARALLELISM: usize = 8;
    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(OPEN_PARALLELISM));
    let mut opens = FuturesUnordered::new();
    for (key, writer) in keys {
        let sem = std::sync::Arc::clone(&sem);
        opens.push(async move {
            let _permit = sem.acquire().await.expect("open semaphore not closed");
            if tokio::time::Instant::now() >= until || crate::join_gate::should_stop(deps) {
                return (
                    key,
                    Err(crate::error::GovernanceRuntimeError::Adapter(
                        "open budget spent before this record's turn".into(),
                    )),
                );
            }
            (key, deps.acquire_record(key, writer.clone()).await)
        });
    }
    let mut taken = Vec::new();
    while let Some((key, result)) = opens.next().await {
        match result {
            Ok(lease) => taken.push((key.clone(), lease)),
            Err(error) => tracing::debug!(
                community = %community_id,
                %key,
                %error,
                "failed to open community record",
            ),
        }
    }
    taken
}

/// Take a SINGLE community's session leases (governance, registry,
/// channel-log and GovernanceOverflow records) and hand them to the host,
/// which watches them and starts the community's loops
/// ([`GovernanceRuntimeDeps::community_records_ready`]).
///
/// Shared by login hydration and the self-sovereign join path
/// (`services::community::join::flow`). The registry is borrowed **with**
/// its writer keypair when one is known; the pool's writer is sticky, so a
/// later read-only borrow never strips it. Best-effort — per-key failures
/// are logged; never short-circuits the caller. A record that failed to
/// open is simply absent from the hand-over.
pub async fn open_and_track_one_community<D: GovernanceRuntimeDeps>(
    deps: &D,
    rec: &CommunityDhtOpenSetup,
) {
    if crate::join_gate::should_stop(deps) {
        return;
    }
    let mut leases = CommunityLeases::default();
    match deps.acquire_record(&rec.governance_key, None).await {
        Ok(lease) => leases.governance = Some(lease),
        Err(error) => {
            tracing::debug!(
                community = %rec.id,
                %error,
                "failed to open governance record",
            );
            return;
        }
    }
    if let Some(reg_key) = &rec.registry_key {
        match deps
            .acquire_record(reg_key, rec.registry_writer.clone())
            .await
        {
            Ok(lease) => leases.registry = Some(lease),
            Err(error) => tracing::warn!(
                community = %rec.id,
                %error,
                "failed to open registry record",
            ),
        }
    }

    // Channel-log + GovernanceOverflow records in ONE bounded concurrent
    // batch, so an unreachable overflow page overlaps the channel opens. The
    // 12 s deadline (for starting borrows only) keeps the best-effort opens
    // from starving the caller (the join's OpenRecords gate, or login
    // hydration). A missing overflow page never aborts the pass (the read
    // path warns and tolerates truncation, D6).
    let channels = deps.channel_log_keys_for_community(&rec.id);
    let overflow_keys = deps.governance_overflow_keys_for_community(&rec.id);
    // Channel records are borrowed writable with our slot writer (they share
    // the registry's slot seed), so a channel write restored after a re-login
    // re-pushes as us (plan C7.13); overflow pages are read-only.
    let mut keys: Vec<(String, Option<String>)> = channels
        .iter()
        .map(|(_, key)| (key.clone(), rec.slot_writer.clone()))
        .collect();
    keys.extend(overflow_keys.iter().map(|key| (key.clone(), None)));
    let until = tokio::time::Instant::now() + std::time::Duration::from_secs(12);
    for (key, lease) in acquire_records_concurrent(deps, &rec.id, &keys, until).await {
        let channel = channels
            .iter()
            .find(|(_, k)| *k == key)
            .and_then(|(id_hex, _)| channel_id_from_hex(id_hex));
        match channel {
            Some(channel_id) => {
                leases.channels.insert(channel_id, lease);
            }
            None => leases.overflow.push(lease),
        }
    }
    deps.community_records_ready(&rec.id, leases).await;
}

/// A channel id from its hex form (16 bytes).
fn channel_id_from_hex(id_hex: &str) -> Option<ChannelId> {
    let bytes: [u8; 16] = hex::decode(id_hex).ok()?.try_into().ok()?;
    Some(ChannelId(bytes))
}

/// Recover per-community registry-linked state from the DHT:
///   1. Read each community's member registry; for the row matching
///      our `my_pseudonym_key`, install `my_subkey_index` +
///      `my_role_ids` on the in-memory `CommunityState` and persist
///      both to SQLite.
///   2. Derive `slot_keypair` immediately if `slot_seed` +
///      `my_subkey_index` are both present (avoids the 60-second
///      presence-poll wait).
///   3. Belt-and-suspenders: recover `registry_owner_keypair` from
///      Stronghold for any community where login didn't load it.
///
/// Best-effort — per-community failures are logged inside the
/// adapter; the orchestrator never short-circuits.
pub async fn hydrate_community_state_from_dht<D: GovernanceRuntimeDeps>(deps: &D) {
    let registry_info = deps.list_registries_with_my_pseudonym();

    for (community_id, registry_key, my_pk) in &registry_info {
        if crate::join_gate::should_stop(deps) {
            return;
        }
        let Some(pk) = my_pk else { continue };
        // Recover our slot by finding the registry row that carries OUR
        // signature, rather than by reading a member-index row that
        // claimed to describe us.
        //
        // Strictly stronger than what it replaces. The index was a
        // shared structure under `o_cnt: 0` — any member could write it,
        // so a row asserting our slot index proved nothing. A presence
        // row must be signed by the pseudonym it names, and only we hold
        // that key.
        //
        // Roles come from the merged CRDT for the same reason: that is
        // where role assignments are authoritative.
        //
        // Defaulted rather than skipped when absent. This runs *before*
        // `rebuild_governance_from_dht`, so on a cold
        // `governance_entries_cache` there is no merged state yet, and
        // refusing to look would lose the slot recovery entirely. The
        // state is only used to filter bans and read roles: an empty ban
        // set means a banned-self row still resolves our slot (which the
        // member index also did, being ban-blind), and empty roles are a
        // documented no-op in `apply_recovered_member_state`.
        let gov_state = deps.governance_state(community_id).unwrap_or_default();
        let Some(subkey) = crate::roster::find_my_slot(deps, registry_key, pk, &gov_state).await
        else {
            continue;
        };
        let role_ids = gov_state
            .role_assignments
            .get(&PseudonymKey::from_hex_lossy(pk))
            .map(|roles| {
                roles
                    .iter()
                    .copied()
                    .map(RoleId::to_legacy_u32)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        deps.apply_recovered_member_state(community_id, subkey, &role_ids);
    }

    for (community_id, _, _) in &registry_info {
        deps.try_derive_slot_keypair_if_ready(community_id);
    }

    for community_id in deps.list_missing_registry_keypairs() {
        deps.recover_registry_keypair_from_keystore(&community_id);
    }

    tracing::info!("hydrated community registry-linked state from DHT");
}

/// Rebuild governance state from SMPL governance records for every
/// joined v2.0 community. Best-effort — per-community failures are
/// logged but never stop the loop.
pub async fn rebuild_governance_from_dht<D: GovernanceRuntimeDeps>(deps: &D) {
    let communities = deps.list_community_governance_targets();

    for (community_id, gov_key_str) in &communities {
        if crate::join_gate::should_stop(deps) {
            return;
        }
        // Identify occupied subkeys via UpdateGet (network-authoritative
        // — local seqs may be empty after a restart). A failed inspect skips
        // the community on this pass: there is no blind 0..255 scan (plan
        // C7.5, step 12e).
        let occupied_subkeys: Vec<u32> =
            match crate::records::inspect_update_get_seqs(deps, gov_key_str).await {
                // `is_some()`, not `!= 0`. A subkey written exactly
                // once sits at seq 0, so the old test skipped every
                // author with a single governance entry — their write
                // never merged on any peer that hydrated cold.
                Ok(seqs) => seqs
                    .iter()
                    .enumerate()
                    .filter(|(_, seq)| seq.is_some())
                    .map(|(idx, _)| u32::try_from(idx).unwrap_or(0))
                    .collect(),
                Err(error) => {
                    tracing::warn!(
                        community = %community_id,
                        %error,
                        "governance inspect failed — skipping this community until the next pass",
                    );
                    continue;
                }
            };

        // Read each occupied subkey, W26-verify, and follow every author's
        // `overflow_next` chain so an author whose log spilled past one subkey
        // is reassembled in full before merge (architecture §"Follow
        // GovernanceOverflow pointers", line 1609). Sequential — the
        // chiral-split move intentionally simplifies the original
        // FuturesUnordered+Semaphore pattern; per-login hydration runs once and
        // the network read cost dominates anyway.
        let readout =
            crate::overflow::read_governance_with_overflow(deps, gov_key_str, &occupied_subkeys)
                .await;
        let all_entries: Vec<(PseudonymKey, Vec<GovernanceEntry>)> = readout.authored;

        // Mutual Aid §14.1 — register every overflow record we just followed in
        // the community's inventory so it is warmed + rehydrated (D5) by this
        // node going forward. Reconstructs the inventory from the chain itself
        // after a restart (the primary subkey + its `overflow_next` are
        // local-store hits), with no SQLite column.
        if !readout.overflow_keys.is_empty() {
            deps.register_governance_overflow_keys(community_id, &readout.overflow_keys);
        }

        if all_entries.is_empty() {
            tracing::debug!(
                community = %community_id,
                "governance record empty — no entries to merge",
            );
            continue;
        }

        let previous_bans = deps
            .governance_state(community_id)
            .map(|gov| gov.bans)
            .unwrap_or_default();

        let (gov_state, accepted_clock) =
            rekindle_governance::merge::merge_with_accepted(&all_entries);
        let new_bans: Vec<String> = gov_state
            .bans
            .iter()
            .filter(|pseudo| !previous_bans.contains(*pseudo))
            .map(|pseudo| hex::encode(pseudo.0))
            .collect();

        // Persist the raw per-author entry set as a warm local cache so the
        // next login can re-merge an identical GovernanceState immediately,
        // instead of waiting on this (slow, best-effort) DHT pass. Done
        // before apply so a crash mid-apply still leaves a usable cache.
        deps.persist_governance_entries_cache(community_id, &all_entries);

        deps.apply_governance_rebuild_result(community_id, gov_state, accepted_clock)
            .await;

        tracing::info!(
            community = %community_id,
            accepted_clock,
            "rebuilt governance state from DHT",
        );

        for banned_pseudonym in new_bans {
            deps.spawn_text_mek_rotation_for_ban(community_id, &banned_pseudonym);
        }
    }
}

/// Re-open every locally-held write-once community record so veilid rehydrates
/// it: an `open_record` local-store hit schedules `add_rehydration_request`,
/// whose background write re-propagates the already-signed local subkey to
/// restore network consensus — no owner keypair required. Covers two record
/// classes whose owner keypair is not retained for live writes:
///
/// 1. **Invite-secrets DFLT records** we authored — the DFLT owner keypair is
///    discarded after the one-shot `publish_invite_secrets`, so without this an
///    inviter restart lets the record age off the DHT and the invite stops
///    resolving for new joiners.
/// 2. **GovernanceOverflow records** this node holds — both the author's own
///    spill pages AND any overflow chain followed as a reader (the inventory is
///    repopulated from the `overflow_next` chain by `rebuild_governance_from_dht`,
///    which runs just before this). Re-opening keeps them on the network across a
///    restart, symmetric for owner and member ("readers keep what they read
///    alive", §14.1) — without it a spilled channel vanishes for joiners once the
///    last holder restarts past the DHT TTL.
///
/// Best-effort — per-key failures are logged, never fatal to login. Runs after
/// `rebuild_governance_from_dht` so `gov_state.invites` + the overflow inventory
/// are populated; both record classes are local-store hits (we authored or
/// previously opened+tracked them).
pub async fn republish_active_records<D: GovernanceRuntimeDeps>(deps: &D) {
    let invite_keys = deps.list_my_active_invite_secret_keys();

    let mut overflow_keys: Vec<String> = Vec::new();
    for (community_id, _gov_key) in deps.list_community_governance_targets() {
        overflow_keys.extend(deps.governance_overflow_keys_for_community(&community_id));
    }
    overflow_keys.sort();
    overflow_keys.dedup();

    let mut rehydrated = 0usize;
    for key in invite_keys.iter().chain(overflow_keys.iter()) {
        if crate::join_gate::should_stop(deps) {
            return;
        }
        // Opening a local record queues its rehydration
        // (`storage_manager/open_record.rs:27-44`), which runs from the
        // local store even after the record closes (`rehydrate.rs:118-131`),
        // so the borrow ends at once.
        match deps.acquire_record(key, None).await {
            Ok(lease) => {
                deps.release_record(lease).await;
                rehydrated += 1;
            }
            Err(error) => tracing::debug!(%key, %error, "record re-open for rehydration failed"),
        }
    }
    tracing::info!(
        rehydrated,
        invite_secrets = invite_keys.len(),
        overflow = overflow_keys.len(),
        "re-opened active community records for rehydration"
    );
}

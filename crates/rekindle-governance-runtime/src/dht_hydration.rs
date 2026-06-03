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

use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

use crate::deps::{CommunityDhtOpenSetup, GovernanceRuntimeDeps};

/// Open governance + registry + channel-log DHT records for every
/// joined community. Best-effort — per-key failures log inside the
/// adapter; the orchestrator never short-circuits.
pub async fn open_community_dht_records<D: GovernanceRuntimeDeps>(deps: &D) {
    let records = deps.list_communities_for_dht_open();
    for rec in &records {
        open_one_community_dht_records(deps, rec).await;
    }
    tracing::info!(
        count = records.len(),
        "opened community DHT records after login"
    );
}

/// Open + track + mark-open + watch a SINGLE community's governance,
/// registry, and channel-log records.
///
/// Login hydration uses this combined wrapper; the self-sovereign join
/// flow instead calls [`open_and_track_one_community`] and
/// [`GovernanceRuntimeDeps::watch_community_records_post_open`] as two
/// separately-gated "dial-in" phases (each with its own timeout).
pub async fn open_one_community_dht_records<D: GovernanceRuntimeDeps>(
    deps: &D,
    rec: &CommunityDhtOpenSetup,
) {
    open_and_track_one_community(deps, rec).await;
    deps.watch_community_records_post_open(&rec.id).await;
}

/// Open a community's channel-log SMPL records concurrently (bounded).
/// Channel opens are best-effort — per-key failures are logged. Run in
/// parallel so a cold join's 2-3 channel opens overlap instead of summing:
/// sequential cold opens, each exhausting the adapter's "not found" backoff
/// ladder, blew the 20 s OpenRecords gate budget.
async fn open_channel_records_concurrent<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    channel_keys: &[String],
) {
    use futures::stream::{FuturesUnordered, StreamExt};

    const OPEN_PARALLELISM: usize = 8;
    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(OPEN_PARALLELISM));
    let mut opens = FuturesUnordered::new();
    for key in channel_keys {
        let sem = std::sync::Arc::clone(&sem);
        opens.push(async move {
            let _permit = sem.acquire().await.expect("open semaphore not closed");
            (key, deps.open_dht_record(key, None).await)
        });
    }
    while let Some((key, result)) = opens.next().await {
        if let Err(error) = result {
            tracing::debug!(
                community = %community_id,
                %key,
                %error,
                "failed to open channel SMPL record",
            );
        }
    }
}

/// Open + track + mark-open (NO watch) a SINGLE community's governance,
/// registry, and channel-log records.
///
/// Shared by login hydration (`open_one_community_dht_records`) and the
/// self-sovereign join path (`services::community::join::flow`) so both
/// follow the identical Veilid open → track → mark-open sequence. The
/// registry is opened **with** its writer keypair when one is known: a
/// read-only (`None`) open clobbers the record's stored writer (Veilid
/// `open_existing_record_locked`), so opening read-only here would
/// silently strip write permission and break subsequent presence / slot
/// writes. Best-effort — per-key failures are logged; never
/// short-circuits the caller.
pub async fn open_and_track_one_community<D: GovernanceRuntimeDeps>(
    deps: &D,
    rec: &CommunityDhtOpenSetup,
) {
    // Governance record (read-only — writes use the shared slot keypair
    // inline at set time).
    if let Err(error) = deps.open_dht_record(&rec.governance_key, None).await {
        tracing::debug!(
            community = %rec.id,
            %error,
            "failed to open governance record",
        );
        return;
    }

    // Registry record — open WITH the writer keypair when we have one so
    // subsequent presence/slot writes go through.
    if let Some(reg_key) = &rec.registry_key {
        let writer = rec.registry_writer.clone();
        if let Err(error) = deps.open_dht_record(reg_key, writer).await {
            tracing::warn!(
                community = %rec.id,
                %error,
                "failed to open registry record",
            );
        }
    }

    // Channel-log records (concurrent, best-effort).
    let channel_keys = deps.channel_log_keys_for_community(&rec.id);
    open_channel_records_concurrent(deps, &rec.id, &channel_keys).await;

    // GovernanceOverflow records (§10 open-once) — best-effort like channels;
    // a missing overflow page never aborts the open pass (the read path warns
    // and tolerates truncation, D6).
    let overflow_keys = deps.governance_overflow_keys_for_community(&rec.id);
    open_channel_records_concurrent(deps, &rec.id, &overflow_keys).await;

    // Track all opened keys + persist the post-open snapshot.
    let mut all_keys = vec![rec.governance_key.clone()];
    if let Some(rk) = &rec.registry_key {
        all_keys.push(rk.clone());
    }
    all_keys.extend(channel_keys.iter().cloned());
    all_keys.extend(overflow_keys.iter().cloned());
    deps.track_open_dht_records(&all_keys);

    deps.mark_community_records_open(
        &rec.id,
        &rec.governance_key,
        rec.registry_key.as_deref(),
        rec.registry_writer.as_deref(),
        channel_keys,
    );
}

/// Join-path open: identical sequence to [`open_and_track_one_community`],
/// but the governance + registry opens are **required** — their failure
/// aborts the caller (the join's OpenRecords gate) with a precise error
/// instead of marking a half-open community that later raises veilid's
/// "record not open". Channel-log opens stay best-effort. Login hydration
/// keeps using the swallowing [`open_and_track_one_community`].
pub async fn try_open_and_track_one_community<D: GovernanceRuntimeDeps>(
    deps: &D,
    rec: &CommunityDhtOpenSetup,
) -> Result<(), String> {
    deps.open_dht_record(&rec.governance_key, None)
        .await
        .map_err(|e| format!("open governance record: {e}"))?;

    if let Some(reg_key) = &rec.registry_key {
        deps.open_dht_record(reg_key, rec.registry_writer.clone())
            .await
            .map_err(|e| format!("open registry record: {e}"))?;
    }

    let channel_keys = deps.channel_log_keys_for_community(&rec.id);
    open_channel_records_concurrent(deps, &rec.id, &channel_keys).await;

    // GovernanceOverflow records (§10) — best-effort; the join's OpenRecords
    // gate must not abort on a missing overflow page (self-sovereign join
    // tolerates incomplete governance — the read path warns and continues).
    let overflow_keys = deps.governance_overflow_keys_for_community(&rec.id);
    open_channel_records_concurrent(deps, &rec.id, &overflow_keys).await;

    let mut all_keys = vec![rec.governance_key.clone()];
    if let Some(rk) = &rec.registry_key {
        all_keys.push(rk.clone());
    }
    all_keys.extend(channel_keys.iter().cloned());
    all_keys.extend(overflow_keys.iter().cloned());
    deps.track_open_dht_records(&all_keys);

    deps.mark_community_records_open(
        &rec.id,
        &rec.governance_key,
        rec.registry_key.as_deref(),
        rec.registry_writer.as_deref(),
        channel_keys,
    );
    Ok(())
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
        let Some(pk) = my_pk else { continue };
        let members = match deps.read_member_index_for_registry(registry_key).await {
            Ok(m) => m,
            Err(error) => {
                tracing::debug!(
                    community = %community_id,
                    %error,
                    "failed to read member registry during hydration",
                );
                continue;
            }
        };
        if let Some(me) = members.iter().find(|m| &m.pseudonym_key_hex == pk) {
            deps.apply_recovered_member_state(community_id, me.subkey_index, &me.role_ids);
        }
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
        // Open the governance record before reading subkeys. May already
        // be open from a previous session; failure here means we'll skip
        // this community on this hydration pass.
        if let Err(error) = deps.open_dht_record(gov_key_str, None).await {
            tracing::debug!(
                community = %community_id,
                %error,
                "failed to open governance record for hydration",
            );
            continue;
        }

        // Identify occupied subkeys via UpdateGet (network-authoritative
        // — local seqs may be empty after a restart).
        let occupied_subkeys: Vec<u32> =
            match deps.inspect_dht_record_update_get_seqs(gov_key_str).await {
                Ok(seqs) => seqs
                    .iter()
                    .enumerate()
                    .filter(|(_, &seq)| seq != 0)
                    .map(|(idx, _)| u32::try_from(idx).unwrap_or(0))
                    .collect(),
                Err(error) => {
                    tracing::warn!(
                        community = %community_id,
                        %error,
                        "governance inspect failed — falling back to full scan",
                    );
                    (0..255_u32).collect()
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

        let gov_state = rekindle_governance::merge::merge(&all_entries);
        let new_bans: Vec<String> = gov_state
            .bans
            .iter()
            .filter(|pseudo| !previous_bans.contains(*pseudo))
            .map(|pseudo| hex::encode(pseudo.0))
            .collect();

        let max_lamport = all_entries
            .iter()
            .flat_map(|(_, entries)| entries.iter().map(GovernanceEntry::lamport))
            .max()
            .unwrap_or(0);

        // Persist the raw per-author entry set as a warm local cache so the
        // next login can re-merge an identical GovernanceState immediately,
        // instead of waiting on this (slow, best-effort) DHT pass. Done
        // before apply so a crash mid-apply still leaves a usable cache.
        deps.persist_governance_entries_cache(community_id, &all_entries);

        deps.apply_governance_rebuild_result(community_id, gov_state, max_lamport)
            .await;

        tracing::info!(
            community = %community_id,
            max_lamport,
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
        match deps.open_dht_record(key, None).await {
            Ok(()) => rehydrated += 1,
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

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

use rekindle_types::governance::{GovernanceEntry, GovernanceSubkeyPayload};
use rekindle_types::id::PseudonymKey;

use crate::deps::GovernanceRuntimeDeps;

/// Open governance + registry + channel-log DHT records for every
/// joined community. Best-effort — per-key failures log inside the
/// adapter; the orchestrator never short-circuits.
pub async fn open_community_dht_records<D: GovernanceRuntimeDeps>(deps: &D) {
    let records = deps.list_communities_for_dht_open();

    for rec in &records {
        // Governance record (read-only — no writer keypair).
        if let Err(error) = deps.open_dht_record(&rec.governance_key, None).await {
            tracing::debug!(
                community = %rec.id,
                %error,
                "failed to open governance record",
            );
            continue;
        }

        // Registry record — open with the writer keypair when we have
        // one so subsequent presence writes go through.
        if let Some(reg_key) = &rec.registry_key {
            let writer = rec.registry_writer.clone();
            if let Err(error) = deps.open_dht_record(reg_key, writer).await {
                tracing::warn!(
                    community = %rec.id,
                    %error,
                    "failed to open registry record on login",
                );
            }
        }

        // Channel-log records.
        let channel_keys = deps.channel_log_keys_for_community(&rec.id);
        for key in &channel_keys {
            if let Err(error) = deps.open_dht_record(key, None).await {
                tracing::debug!(
                    community = %rec.id,
                    %key,
                    %error,
                    "failed to open channel SMPL record on login",
                );
            }
        }

        // Track all opened keys + persist the post-open snapshot.
        let mut all_keys = vec![rec.governance_key.clone()];
        if let Some(rk) = &rec.registry_key {
            all_keys.push(rk.clone());
        }
        all_keys.extend(channel_keys.iter().cloned());
        deps.track_open_dht_records(&all_keys);

        deps.mark_community_records_open(
            &rec.id,
            &rec.governance_key,
            rec.registry_key.as_deref(),
            rec.registry_writer.as_deref(),
            channel_keys,
        );

        deps.watch_community_records_post_open(&rec.id).await;
    }

    tracing::info!(count = records.len(), "opened community DHT records after login");
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

        // Read each occupied subkey. Sequential — the chiral-split move
        // intentionally simplifies the original FuturesUnordered+Semaphore
        // pattern; per-login hydration runs once and the network read
        // cost dominates anyway.
        let mut all_entries: Vec<(PseudonymKey, Vec<GovernanceEntry>)> = Vec::new();
        for subkey in occupied_subkeys {
            let bytes = match deps.get_dht_value(gov_key_str, subkey, false).await {
                Ok(Some(b)) => b,
                Ok(None) => continue,
                Err(error) => {
                    tracing::debug!(
                        community = %community_id,
                        subkey,
                        %error,
                        "failed to read governance subkey",
                    );
                    continue;
                }
            };
            if bytes.is_empty() {
                continue;
            }
            let Ok(payload) = serde_json::from_slice::<GovernanceSubkeyPayload>(&bytes) else {
                continue;
            };
            // Architecture §26 W26 — drop subkey reads whose author
            // signature doesn't verify. The SMPL slot keypair is
            // community-shared, so any member could otherwise forge a
            // payload claiming to be the creator.
            let Ok(sig_arr): Result<[u8; 64], _> = payload.signature.as_slice().try_into() else {
                continue;
            };
            if rekindle_secrets::derive::verify_pseudonym_signature(
                &payload.author_pseudonym.0,
                &payload.signing_bytes(),
                &sig_arr,
            )
            .is_err()
            {
                tracing::warn!(
                    community = %community_id,
                    "governance subkey rejected: bad pseudonym signature",
                );
                continue;
            }
            all_entries.push((payload.author_pseudonym, payload.entries));
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

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

use crate::event::GovernanceRuntimeEvent;
use rekindle_records::lease::{CommunityLeases, LeaseId};
use rekindle_types::id::{ChannelId, PseudonymKey, RoleId};

use crate::deps::{CommunityDhtOpenSetup, GovernanceRuntimeDeps};

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

/// One community's member state from the DHT (plan E3.1 C2c: the Host's
/// per-community login stage, which a reconnect or a wake re-runs): our slot and roles from its registry, its slot keypair, and
/// its registry keypair from the vault when login did not load it.
pub async fn hydrate_one_community<D: GovernanceRuntimeDeps>(deps: &D, community: &str) {
    let registry = deps
        .list_registries_with_my_pseudonym()
        .into_iter()
        .find(|(id, _, _)| id == community);
    if let Some((community_id, registry_key, Some(pk))) = &registry {
        let gov_state = deps.governance_state(community_id).unwrap_or_default();
        if let Some(subkey) = crate::roster::find_my_slot(deps, registry_key, pk, &gov_state).await
        {
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
    }
    if registry.is_some() {
        deps.try_derive_slot_keypair_if_ready(community);
    }
    if deps
        .list_missing_registry_keypairs()
        .iter()
        .any(|id| id == community)
    {
        deps.recover_registry_keypair_from_keystore(community);
    }
}

/// Rebuild one community's governance state from its op set (plan E3.2):
/// `op_log::rebuild_ops` re-reads `governance_ops`, fetches missing deps by
/// `OpRef`, and merges under the pinned `GenesisCert`. The watch path passes
/// the community whose records changed, so the others are not re-read.
/// Best-effort — a failure is logged and the community keeps its state
/// until its next change.
pub async fn rebuild_governance<D: GovernanceRuntimeDeps>(deps: &D, community: &str) {
    if !deps
        .list_community_governance_targets()
        .iter()
        .any(|(id, _)| id == community)
    {
        tracing::debug!(community, "not a joined community — no governance rebuild");
        return;
    }
    let community_id = &community.to_owned();
    let previous_bans = deps
        .governance_state(community_id)
        .map(|gov| gov.bans)
        .unwrap_or_default();
    let gov_state = match crate::op_log::rebuild_ops(deps, community_id).await {
        Ok(state) => state,
        Err(error) => {
            tracing::warn!(community = %community_id, %error, "governance rebuild failed");
            return;
        }
    };
    let new_bans: Vec<String> = gov_state
        .bans
        .iter()
        .filter(|pseudo| !previous_bans.contains(*pseudo))
        .map(|pseudo| hex::encode(pseudo.0))
        .collect();
    deps.apply_governance_rebuild_result(community_id, gov_state)
        .await;
    deps.emit_event(GovernanceRuntimeEvent::GovernanceRebuilt {
        community_id: community_id.clone(),
    });

    tracing::info!(community = %community_id, "rebuilt governance state from the op set");
    crate::member_card::refresh_pending_cards(deps, community_id).await;

    for banned_pseudonym in new_bans {
        deps.spawn_text_mek_rotation_for_ban(community_id, &banned_pseudonym);
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
///    repopulated from the `overflow_next` chain by `rebuild_governance`,
///    which runs just before this). Re-opening keeps them on the network across a
///    restart, symmetric for owner and member ("readers keep what they read
///    alive", §14.1) — without it a spilled channel vanishes for joiners once the
///    last holder restarts past the DHT TTL.
///
/// Best-effort — per-key failures are logged, never fatal to login. Runs after
/// each community's `rebuild_governance` so `gov_state.invites` + the overflow inventory
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

//! Phase 23.D.11 — rotator-initiated MEK rotation orchestration
//! ported from `src-tauri/services/community/mek_rotation_orchestrators.rs`.
//!
//! `rotate_text_mek_for_departure` runs the cascade-elected rotator's
//! full pipeline: cascade-slot wait → MEK generation → distribute via
//! `distribute_mek` → cache insert → MEKGenerationBump governance entry
//! → mesh broadcast of `MEKRotated`.
//!
//! `rotate_voice_mek_for_membership` is the channel-scoped variant
//! (no governance entry — voice MEK rotations skip CRDT and rely on
//! the gossip MEKRotated broadcast for membership notification).

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_types::governance::GovernanceEntry;

use crate::deps::MekDistributeDeps;
use crate::distribute::distribute_mek;
use crate::election::{cascade_candidates, MAX_CASCADES};
use crate::error::MekRotationError;
use crate::{wait_for_rotation_slot, RotationRecipient};

use crate::pseudonym_hex::{pseudonym_from_hex, pseudonym_hex};

pub async fn rotate_text_mek_for_departure<D: MekDistributeDeps>(
    deps: &D,
    community_id: &str,
    departed_pseudonym: &str,
) -> Result<(), MekRotationError> {
    let departed = pseudonym_from_hex(departed_pseudonym)
        .ok_or_else(|| MekRotationError::InvalidInput("invalid departed pseudonym".to_string()))?;
    let recipients = deps.online_recipients(community_id, Some(departed_pseudonym));
    let candidate_keys = recipients
        .iter()
        .filter_map(|r| pseudonym_from_hex(&r.pseudonym_hex))
        .collect::<Vec<_>>();
    let candidates = cascade_candidates(&departed, &candidate_keys, MAX_CASCADES);

    let cache = deps.cache();
    let initial_generation = cache.current_generation(community_id, "");
    let Some(cascade_skipped) =
        wait_for_rotation_slot(deps, community_id, None, &candidates, initial_generation).await
    else {
        return Ok(());
    };

    let new_generation = initial_generation + 1;
    // Stamp the minter's deterministic election rank (blake3(departed||me) —
    // the same value `cascade_candidates` ranks by) so every peer converges on
    // the lowest-rank (= rightful primary rotator) key for this generation,
    // resolving same-generation split-brain. Context = `departed` to match the
    // election at the top of this fn.
    let mek = match deps.my_pseudonym(community_id) {
        Some(me) => {
            let rank = rekindle_secrets::rotator::election_hash(&departed.0, &me.0);
            MediaEncryptionKey::generate(new_generation).with_provenance(me.0, rank)
        }
        None => MediaEncryptionKey::generate(new_generation),
    };
    distribute_mek(
        deps,
        community_id,
        None,
        &mek,
        &recipients
            .iter()
            .map(|r| RotationRecipient {
                pseudonym_hex: r.pseudonym_hex.clone(),
                route_blob: r.route_blob.clone(),
            })
            .collect::<Vec<_>>(),
    )
    .await?;

    deps.apply_received_mek_to_state(community_id, None, &mek);
    deps.persist_received_mek(community_id, None, &mek);

    let lamport = deps.increment_lamport(community_id);
    deps.write_governance_entry(
        community_id,
        GovernanceEntry::MEKGenerationBump {
            generation: new_generation,
            trigger_departed: departed,
            cascade_skipped,
            lamport,
        },
    )
    .await?;

    deps.emit_rotation_received(community_id, None, new_generation);
    let rotator_pseudonym = deps.my_pseudonym(community_id).map(|p| pseudonym_hex(&p));
    deps.send_to_mesh(
        community_id,
        &CommunityEnvelope::Control(ControlPayload::MEKRotated {
            channel_id: None,
            new_generation,
            rotator_pseudonym,
        }),
    )?;
    Ok(())
}

pub async fn rotate_voice_mek_for_membership<D: MekDistributeDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    trigger_pseudonym: &str,
    include_trigger_in_recipients: bool,
) -> Result<(), MekRotationError> {
    let trigger = pseudonym_from_hex(trigger_pseudonym)
        .ok_or_else(|| MekRotationError::InvalidInput("invalid trigger pseudonym".to_string()))?;
    let recipients = deps
        .voice_recipients(
            community_id,
            channel_id,
            trigger_pseudonym,
            include_trigger_in_recipients,
        )
        .await;
    let candidate_keys = recipients
        .iter()
        .filter(|r| r.pseudonym_hex != trigger_pseudonym)
        .filter_map(|r| pseudonym_from_hex(&r.pseudonym_hex))
        .collect::<Vec<_>>();
    let candidates = cascade_candidates(&trigger, &candidate_keys, MAX_CASCADES);

    let cache = deps.cache();
    let initial_generation = cache.current_generation(community_id, channel_id);
    let Some(_cascade_skipped) = wait_for_rotation_slot(
        deps,
        community_id,
        Some(channel_id),
        &candidates,
        initial_generation,
    )
    .await
    else {
        return Ok(());
    };

    let new_generation = initial_generation + 1;
    // Stamp the minter's election rank (context = `trigger`, matching the
    // election above) for deterministic same-generation convergence.
    let mek = match deps.my_pseudonym(community_id) {
        Some(me) => {
            let rank = rekindle_secrets::rotator::election_hash(&trigger.0, &me.0);
            MediaEncryptionKey::generate(new_generation).with_provenance(me.0, rank)
        }
        None => MediaEncryptionKey::generate(new_generation),
    };
    distribute_mek(
        deps,
        community_id,
        Some(channel_id),
        &mek,
        &recipients
            .iter()
            .map(|r| RotationRecipient {
                pseudonym_hex: r.pseudonym_hex.clone(),
                route_blob: r.route_blob.clone(),
            })
            .collect::<Vec<_>>(),
    )
    .await?;

    deps.apply_received_mek_to_state(community_id, Some(channel_id), &mek);
    deps.persist_received_mek(community_id, Some(channel_id), &mek);
    deps.emit_rotation_received(community_id, Some(channel_id), new_generation);

    let rotator_pseudonym = deps.my_pseudonym(community_id).map(|p| pseudonym_hex(&p));
    deps.send_to_mesh(
        community_id,
        &CommunityEnvelope::Control(ControlPayload::MEKRotated {
            channel_id: Some(channel_id.to_string()),
            new_generation,
            rotator_pseudonym,
        }),
    )?;
    Ok(())
}

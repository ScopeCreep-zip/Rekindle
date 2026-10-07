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
use rekindle_types::channel_keys::KeyScope;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::ChannelId;

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
    // The rotator is elected only among members who may rotate (plan
    // D20) — every peer filters the same merged governance, so all agree
    // on the candidates. With none online, nobody rotates: the rotation
    // waits for an eligible member rather than falling to one readers
    // would refuse.
    let mut candidate_keys = recipients
        .iter()
        .filter_map(|r| pseudonym_from_hex(&r.pseudonym_hex))
        .collect::<Vec<_>>();
    if let Some(me) = deps.my_pseudonym(community_id) {
        if !candidate_keys.contains(&me) {
            candidate_keys.push(me);
        }
    }
    candidate_keys.retain(|candidate| deps.may_rotate(community_id, candidate));
    if candidate_keys.is_empty() {
        tracing::info!(community = %community_id, "departure rotation pending — no member who may rotate is online");
        return Ok(());
    }
    let candidates = cascade_candidates(&departed, &candidate_keys, MAX_CASCADES);

    let scope = KeyScope::Community;
    let initial_generation = deps.cache().current_generation(community_id, scope);
    let Some(cascade_skipped) =
        wait_for_rotation_slot(deps, community_id, scope, &candidates, initial_generation).await
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
        scope,
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

    install_minted(deps, community_id, scope, &mek)?;

    let lamport = deps.next_governance_lamport(community_id)?;
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

    deps.emit_rotation_received(community_id, scope, new_generation);
    let rotator_pseudonym = deps.my_pseudonym(community_id).map(|p| pseudonym_hex(&p));
    deps.send_to_mesh(
        community_id,
        &CommunityEnvelope::Control(ControlPayload::MEKRotated {
            channel_id: scope.wire_channel(),
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
    let channel = ChannelId::from_hex(channel_id).ok_or_else(|| {
        MekRotationError::InvalidInput(format!("invalid channel id {channel_id}"))
    })?;
    let scope = KeyScope::Channel(channel);
    let recipients = deps
        .voice_recipients(
            community_id,
            channel,
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

    let initial_generation = deps.cache().current_generation(community_id, scope);
    let Some(_cascade_skipped) =
        wait_for_rotation_slot(deps, community_id, scope, &candidates, initial_generation).await
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
        scope,
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

    install_minted(deps, community_id, scope, &mek)?;
    deps.emit_rotation_received(community_id, scope, new_generation);

    let rotator_pseudonym = deps.my_pseudonym(community_id).map(|p| pseudonym_hex(&p));
    deps.send_to_mesh(
        community_id,
        &CommunityEnvelope::Control(ControlPayload::MEKRotated {
            channel_id: scope.wire_channel(),
            new_generation,
            rotator_pseudonym,
        }),
    )?;
    Ok(())
}

/// Replace a MEK because an operator asked, not because somebody left.
///
/// `scope` is the community key or one channel's.
///
/// [`rotate_text_mek_for_departure`] cannot serve this: it elects a
/// rotator from `blake3(departed || candidate)` and there is no departed
/// member to seed that with. The request names *this* node, so it
/// rotates and distributes directly, with no cascade wait.
///
/// Both shells drive this. The desktop's `rotate_mek_local` and the
/// daemon's `governance_rpc::rekey` each did it their own way, and both
/// did it by publishing wrapped copies into the registry's MEK vault
/// subkey — a write `o_cnt: 0` grants nobody a credential for, of a key
/// `communities-channels.md` says is *"**never** written to DHT"*. The
/// desktop's even required a `registry_owner_keypair` and refused
/// without one, which is the coordinator in miniature. Delivery here is
/// per-recipient `app_call`, the same path a departure rotation uses.
///
/// Only a member who may rotate (KICK, BAN or MANAGE_COMMUNITY) may do
/// this; honest peers accept the resulting `MEKGenerationBump` only from
/// such a member and only as `current + 1` (plan D20).
pub async fn rotate_mek_on_request<D: MekDistributeDeps>(
    deps: &D,
    community_id: &str,
    scope: KeyScope,
) -> Result<(), MekRotationError> {
    // Resolved before any work: the bump entry has to name a real
    // pseudonym, and a placeholder would merge as though an unrelated
    // member had departed.
    let me = deps
        .my_pseudonym(community_id)
        .ok_or_else(|| MekRotationError::PseudonymMissing(community_id.to_string()))?;
    // An operator rotation is the same act as a departure rotation, so it
    // takes the same permission (plan D20) — readers would refuse the
    // bump otherwise.
    if !deps.may_rotate(community_id, &me) {
        return Err(MekRotationError::NotPermitted);
    }
    let me_hex = pseudonym_hex(&me);

    let new_generation = deps.cache().current_generation(community_id, scope) + 1;

    // Provenance is what makes two admins rotating at the same
    // generation converge: `convergence::incoming_wins_same_generation`
    // breaks the tie on the lowest election rank, and every peer
    // computes the same answer. Without it the two keys are
    // indistinguishable and peers split into two decryptable halves.
    // A manual rotation has no trigger member, so the rank is taken
    // against a zero context — a stable per-minter tiebreak.
    let rank = rekindle_secrets::rotator::election_hash(&[0u8; 32], &me.0);
    let new_mek = MediaEncryptionKey::generate(new_generation).with_provenance(me.0, rank);

    // Excluding nobody: everyone online is still a member.
    let recipients = deps.online_recipients(community_id, None);
    distribute_mek(deps, community_id, scope, &new_mek, &recipients).await?;

    install_minted(deps, community_id, scope, &new_mek)?;

    // Stamp the community generation so peers that were offline during
    // the distribution can tell their cached key is stale. A channel key's
    // generation is its own and is not a governance fact.
    if scope == KeyScope::Community {
        let lamport = deps.next_governance_lamport(community_id)?;
        deps.write_governance_entry(
            community_id,
            GovernanceEntry::MEKGenerationBump {
                generation: new_generation,
                // No departure triggered this, so the field names the
                // initiator rather than an uninvolved member who would
                // otherwise look like they had left.
                trigger_departed: me,
                cascade_skipped: Vec::new(),
                lamport,
            },
        )
        .await?;
    }

    deps.send_to_mesh(
        community_id,
        &CommunityEnvelope::Control(ControlPayload::MEKRotated {
            channel_id: scope.wire_channel(),
            new_generation,
            rotator_pseudonym: Some(me_hex),
        }),
    )
}

/// Install and persist a key this node just minted and distributed. A
/// refusal means a competing key won the same-generation tiebreak while we
/// were distributing; announcing ours after that would split the scope.
fn install_minted<D: MekDistributeDeps>(
    deps: &D,
    community_id: &str,
    scope: KeyScope,
    mek: &MediaEncryptionKey,
) -> Result<(), MekRotationError> {
    if !deps.apply_received_mek_to_state(community_id, scope, mek) {
        return Err(MekRotationError::InvalidInput(format!(
            "minted {scope} generation {} lost to a competing key",
            mek.generation()
        )));
    }
    deps.persist_received_mek(community_id, scope, mek);
    Ok(())
}

/// Mint a voice channel's first key when we join it holding none
/// (plan B5.4). A sender holds its key before it sends — Matrix creates
/// its outbound session, WhatsApp its sender key, on first send — and a
/// channel's media is under its own key (architecture §10.5), never the
/// community key. Returns whether a key was minted.
///
/// Distribution is not needed: members already present rotate on our
/// join and wrap their next key to us, which supersedes this one; two
/// members minting at once converge on the same-generation tiebreak.
pub fn mint_first_channel_key<D: MekDistributeDeps>(
    deps: &D,
    community_id: &str,
    channel: ChannelId,
) -> Result<bool, MekRotationError> {
    let scope = KeyScope::Channel(channel);
    if deps.cache().current_generation(community_id, scope) > 0 {
        return Ok(false);
    }
    let me = deps
        .my_pseudonym(community_id)
        .ok_or_else(|| MekRotationError::PseudonymMissing(community_id.to_string()))?;
    let rank = rekindle_secrets::rotator::election_hash(&channel_context(channel), &me.0);
    let mek = MediaEncryptionKey::generate(1).with_provenance(me.0, rank);
    install_minted(deps, community_id, scope, &mek)?;
    deps.emit_rotation_received(community_id, scope, 1);
    Ok(true)
}

/// The election context for a channel's first key: its id, so concurrent
/// first mints rank against the same value on every peer.
fn channel_context(channel: ChannelId) -> [u8; 32] {
    let mut context = [0u8; 32];
    context[..16].copy_from_slice(&channel.0);
    context
}

//! `JoinAccepted` control-message handler (drained from
//! `legacy/membership/join.rs`). Caches the MEK, recovers our slot
//! index, persists the member list, derives the slot keypair from a
//! granted seed, and kicks off peer bootstrap.

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_protocol::dht::community::types::MemberSummary;
use rekindle_secrets::derive;

use crate::deps::MekSnapshot;
use crate::event::GovernanceRuntimeEvent;
use crate::membership_events::deps::{MemberUpsertRow, MembershipEventDeps, SlotGrantUpdate};
use crate::membership_events::grants::unwrap_grant;

/// Borrowed view of a `ControlPayload::JoinAccepted` payload. Mirrors the
/// legacy `join_accepted_data` constructor so the adapter entry point can
/// build it directly from the decoded `ControlPayload`.
pub struct JoinAcceptedInput<'a> {
    pub mek_wire_bytes: &'a [u8],
    pub mek_generation: u64,
    pub members: &'a [MemberSummary],
    pub member_registry_key: Option<&'a str>,
    pub slot_index: Option<u32>,
    pub wrapped_slot_seed: Option<&'a [u8]>,
}

fn member_rows(members: &[MemberSummary]) -> Vec<MemberUpsertRow> {
    members
        .iter()
        .map(|m| MemberUpsertRow {
            pseudonym_hex: m.pseudonym_key.clone(),
            display_name: m.display_name.clone(),
            role_ids: m.role_ids.clone(),
            joined_at: m.joined_at,
            subkey_index: m.subkey_index,
            onboarding_complete: m.onboarding_complete,
            timeout_until: m.timeout_until,
        })
        .collect()
}

pub async fn process_join_accepted<D: MembershipEventDeps>(
    deps: &D,
    community_id: &str,
    sender_pseudonym: &str,
    input: JoinAcceptedInput<'_>,
) {
    let my_pseudonym = deps
        .community_membership(community_id)
        .and_then(|m| m.my_pseudonym_hex);
    if my_pseudonym.as_deref() == Some(sender_pseudonym) {
        tracing::debug!(community = %community_id, "ignoring self-JoinAccepted loopback");
        return;
    }

    let has_mek = !input.mek_wire_bytes.is_empty();
    if has_mek {
        if let Some(mek) = MediaEncryptionKey::from_wire_bytes(input.mek_wire_bytes) {
            let generation = mek.generation();
            deps.insert_community_mek(
                community_id,
                MekSnapshot {
                    generation,
                    key_bytes: *mek.as_bytes(),
                },
            );
            tracing::info!(community = %community_id, mek_generation = generation, "cached MEK from JoinAccepted");
        } else {
            tracing::warn!(community = %community_id, "JoinAccepted contained invalid MEK wire bytes");
        }
    }

    // Recover my_subkey_index from the member list (backup path). SQLite
    // persist only when the accept carried no explicit slot_index.
    if let Some(my_pk) = my_pseudonym.as_deref() {
        if let Some(me) = input.members.iter().find(|m| m.pseudonym_key == my_pk) {
            deps.backfill_my_subkey_index(
                community_id,
                me.subkey_index,
                input.slot_index.is_none(),
            );
        }
    }

    deps.set_mek_generation_and_registry(
        community_id,
        input.mek_generation,
        input.member_registry_key,
    )
    .await;

    let rows = member_rows(input.members);
    if !rows.is_empty() {
        deps.upsert_members(community_id, rows.clone()).await;
        let pseudonyms: Vec<String> = rows.iter().map(|r| r.pseudonym_hex.clone()).collect();
        deps.insert_known_members(community_id, &pseudonyms);
    }

    if has_mek {
        deps.persist_community_mek_to_keystore(community_id);
    }

    if let (Some(idx), Some(wrapped_seed)) = (input.slot_index, input.wrapped_slot_seed) {
        apply_slot_seed_grant(deps, community_id, sender_pseudonym, idx, wrapped_seed);
    }

    deps.emit_event(GovernanceRuntimeEvent::JoinAccepted {
        community_id: community_id.to_string(),
    });

    tracing::info!(
        community = %community_id,
        mek_generation = input.mek_generation,
        member_count = rows.len(),
        has_slot_keypair = input.slot_index.is_some(),
        "JoinAccepted processed — join state updated"
    );

    deps.spawn_peer_bootstrap(community_id, rows);
}

/// Derive our slot keypair locally from a granted seed (legacy
/// `handle_slot_seed_grant`).
fn apply_slot_seed_grant<D: MembershipEventDeps>(
    deps: &D,
    community_id: &str,
    sender_pseudonym: &str,
    slot_index: u32,
    wrapped_slot_seed: &[u8],
) {
    let Some(seed_bytes) = unwrap_grant(
        deps,
        community_id,
        sender_pseudonym,
        wrapped_slot_seed,
        "slot seed",
    ) else {
        return;
    };
    let seed_hex = String::from_utf8_lossy(&seed_bytes).to_string();

    let Ok(seed_raw) = hex::decode(&seed_hex) else {
        tracing::warn!("slot seed is not valid hex");
        return;
    };
    let Ok(seed_array): Result<[u8; 32], _> = seed_raw.try_into() else {
        tracing::warn!("slot seed wrong length (expected 32 bytes)");
        return;
    };
    let slot_kp = match derive::derive_slot_keypair(&seed_array, slot_index) {
        Ok(sk) => deps.format_writer_keypair(sk.verifying_key().to_bytes(), sk.to_bytes()),
        Err(e) => {
            tracing::warn!(error = %e, slot_index, "failed to derive slot keypair from seed");
            return;
        }
    };

    deps.apply_slot_grant(
        community_id,
        SlotGrantUpdate {
            slot_seed_hex: Some(seed_hex),
            slot_keypair: Some(slot_kp),
            my_subkey_index: Some(slot_index),
            ..SlotGrantUpdate::default()
        },
    );

    tracing::info!(community = %community_id, slot_index, "slot seed received — derived slot keypair locally");

    deps.spawn_presence_poll_tick(community_id);
}

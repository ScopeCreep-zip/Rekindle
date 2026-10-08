//! Keypair-grant control-message handlers (drained from
//! `legacy/membership/grants.rs`).

use rekindle_crypto::group::mek_distribution::unwrap_mek;
use rekindle_crypto::group::pseudonym::derive_community_pseudonym;

use crate::membership_events::deps::{MembershipEventDeps, SlotGrantUpdate};

/// Unwrap a sealed blob addressed to us within `community_id`, sent by
/// `sender_pseudonym`. Returns `None` (with a warn) on any failure.
/// Shared with the JoinAccepted slot-seed path.
pub(crate) fn unwrap_grant<D: MembershipEventDeps>(
    deps: &D,
    community_id: &str,
    sender_pseudonym: &str,
    wrapped: &[u8],
    what: &str,
) -> Option<Vec<u8>> {
    let Some(secret) = deps.identity_secret() else {
        tracing::warn!("no identity secret — cannot unwrap {what}");
        return None;
    };
    let my_signing_key = derive_community_pseudonym(&secret, community_id);

    let Ok(sender_bytes) = hex::decode(sender_pseudonym) else {
        tracing::warn!("invalid sender pseudonym hex in {what}");
        return None;
    };
    let Ok(sender_pub): Result<[u8; 32], _> = sender_bytes.try_into() else {
        tracing::warn!("sender pseudonym wrong length in {what}");
        return None;
    };

    match unwrap_mek(&my_signing_key, &sender_pub, wrapped) {
        Ok(b) => Some(b),
        Err(e) => {
            tracing::warn!(error = %e, "failed to unwrap {what}");
            None
        }
    }
}

/// Admin grants us the community owner keypair + slot seed (admin
/// transfer / co-admin). Installs the owner keypair + seed.
pub fn process_admin_keypair_grant<D: MembershipEventDeps>(
    deps: &D,
    community_id: &str,
    sender_pseudonym: &str,
    wrapped_owner_keypair: &[u8],
    wrapped_slot_seed: &[u8],
) {
    let Some(owner_kp_bytes) = unwrap_grant(
        deps,
        community_id,
        sender_pseudonym,
        wrapped_owner_keypair,
        "community owner keypair",
    ) else {
        return;
    };
    let owner_kp_str = String::from_utf8_lossy(&owner_kp_bytes).to_string();

    let Some(slot_seed_bytes) = unwrap_grant(
        deps,
        community_id,
        sender_pseudonym,
        wrapped_slot_seed,
        "slot seed",
    ) else {
        return;
    };

    deps.apply_slot_grant(
        community_id,
        SlotGrantUpdate {
            dht_owner_keypair: Some(owner_kp_str),
            slot_seed_hex: Some(hex::encode(&slot_seed_bytes)),
            ..SlotGrantUpdate::default()
        },
    );

    tracing::info!(community = %community_id, "community owner keypair grant accepted and persisted");
}

/// Admin grants us a pre-derived slot keypair for a specific slot.
pub fn process_slot_keypair_grant<D: MembershipEventDeps>(
    deps: &D,
    community_id: &str,
    sender_pseudonym: &str,
    slot_index: u32,
    segment_index: u32,
    wrapped_slot_keypair: &[u8],
) {
    let Some(slot_kp_bytes) = unwrap_grant(
        deps,
        community_id,
        sender_pseudonym,
        wrapped_slot_keypair,
        "slot keypair",
    ) else {
        return;
    };
    let slot_kp_str = String::from_utf8_lossy(&slot_kp_bytes).to_string();

    deps.apply_slot_grant(
        community_id,
        SlotGrantUpdate {
            slot_keypair: Some(slot_kp_str),
            my_subkey_index: Some(slot_index),
            ..SlotGrantUpdate::default()
        },
    );

    tracing::info!(
        community = %community_id,
        slot_index, segment_index,
        "slot keypair grant accepted and persisted"
    );

    deps.spawn_presence_poll_tick(community_id);
}

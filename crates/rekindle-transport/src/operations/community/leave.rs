//! Leave a community: release our registry slot, drop the cached MEKs,
//! and build the gossip `MemberLeave` payload for the caller to
//! broadcast.
//!
//! Leaving is unilateral here, which is a deliberate divergence from
//! MLS. RFC 9750 states "there is no similarly unilateral way for a
//! member to leave the group; they must be removed by a remaining
//! member" — because in MLS the group secret is derived from the
//! membership tree, so a departure that nobody commits leaves the key
//! schedule inconsistent. Our registry is a presence directory, not key
//! material: the only thing this releases is the leaver\'s own slot, and
//! W26 proves they authored it. The half that genuinely needs the
//! remaining members is the MEK rotation, which is exactly what
//! `rotate_text_mek_for_departure` does.

use parking_lot::RwLock;
use std::sync::Arc;
use tracing::info;

use super::LeaveResult;
use crate::broadcast::node::TransportNode;
use crate::crypto::mek::MekCache;
use crate::error::{Result, TransportError};

pub async fn leave_community(
    node: &TransportNode,
    membership: &crate::session::CommunityMembership,
    mek_cache: &Arc<RwLock<MekCache>>,
    signing_key_bytes: &[u8; 32],
) -> Result<LeaveResult> {
    info!(community = %membership.community_name, "leaving community via DHT");

    // Release our registry slot by writing a *signed* departure row to
    // the subkey we own.
    //
    // This replaces a `PendingJoinStatus::Left` write into the join
    // inbox — coordinator-era plumbing whose reader went with the v1.0
    // join, so it had become a write nothing consumed. It also delivers
    // what `communities-governance.md` promises and nothing implemented:
    // "zero own registry slot… The slot becomes available for reuse."
    //
    // It has to be signed rather than zeroed. The slot seed is shared
    // with every member, so any member can write any slot; an unsigned
    // empty payload would let anyone free anyone's slot and collide two
    // members onto one index. W26 makes this provably ours.
    //
    // Best-effort: a member leaving must not be blocked by a DHT write,
    // and a slot that fails to release is the status quo, not a
    // regression. `rekindle-mek-rotation` handles the half that does
    // need the remaining members — rotating the key we still hold.
    if let Err(e) = release_registry_slot(node, membership, signing_key_bytes).await {
        tracing::warn!(
            community = %membership.community_name,
            error = %e,
            "could not release registry slot on leave — it stays occupied until reclaimed"
        );
    }

    mek_cache
        .write()
        .remove_community(&membership.governance_key);
    info!(community = %membership.community_name, "community left");
    Ok(LeaveResult {
        departure_notice: rekindle_codec::community::envelope::CommunityEnvelope::Control(
            rekindle_codec::community::envelope::ControlPayload::MemberLeave {
                pseudonym_key: membership.pseudonym_key.clone(),
            },
        ),
    })
}

/// Write the tombstone; a supersede is acted on, never reported as released
/// (plan C7.17). Over our own newer row (the network held a copy our local
/// store missed) it writes once more, above it; over another member's row
/// or an unverifiable value the slot is not ours to release.
async fn write_tombstone(
    node: &TransportNode,
    lease: crate::broadcast::dht_writes::LeaseId,
    subkey: u32,
    bytes: Vec<u8>,
    writer: &str,
    my_pseudonym_hex: &str,
) -> Result<()> {
    use rekindle_codec::presence_row::{classify_superseding_row, SupersedingRow};
    let first = crate::broadcast::dht_writes::set_leased_str(
        node,
        lease,
        subkey,
        bytes.clone(),
        Some(writer),
    )
    .await?;
    let Some(newer) = first else {
        return Ok(());
    };
    match classify_superseding_row(&newer.data, my_pseudonym_hex) {
        SupersedingRow::Ours => {
            match crate::broadcast::dht_writes::set_leased_str(
                node,
                lease,
                subkey,
                bytes,
                Some(writer),
            )
            .await?
            {
                None => Ok(()),
                Some(again) => Err(TransportError::DhtError {
                    reason: format!(
                        "departure tombstone superseded twice (seq {:?}, then {:?})",
                        newer.seq, again.seq
                    ),
                }),
            }
        }
        SupersedingRow::Member(author) => Err(TransportError::DhtError {
            reason: format!(
                "slot {subkey} holds member {author}'s row; departure tombstone not written"
            ),
        }),
        SupersedingRow::Unverified(reason) => Err(TransportError::DhtError {
            reason: format!(
                "slot {subkey} holds an unverified value ({reason}); departure tombstone not written"
            ),
        }),
    }
}

/// Write a signed `departed` row into our own registry subkey.
async fn release_registry_slot(
    node: &TransportNode,
    membership: &crate::session::CommunityMembership,
    signing_key_bytes: &[u8; 32],
) -> Result<()> {
    let Some(slot_seed) = membership.slot_seed else {
        return Err(TransportError::Internal(
            "no slot seed — cannot derive the slot keypair to release".into(),
        ));
    };

    let slot_kp = rekindle_protocol::dht::community::member_registry::derive_slot_veilid_keypair(
        &slot_seed,
        membership.slot_index,
    )
    .map_err(|e| TransportError::Internal(format!("slot keypair derivation failed: {e}")))?;

    let pseudonym = crate::crypto::pseudonym::derive_community_pseudonym(
        signing_key_bytes,
        &membership.governance_key,
    );
    let bytes =
        rekindle_codec::presence_row::departure_row(&pseudonym, rekindle_utils::timestamp_secs());
    let my_pseudonym_hex = hex::encode(pseudonym.verifying_key().to_bytes());

    // A table hit while the community holds its registry (the slot writer
    // is already its sticky writer); the write names the slot keypair
    // either way, and a miss is an error, never a queued write.
    let lease = crate::broadcast::dht_writes::acquire_str(
        node,
        &membership.registry_key,
        Some(&slot_kp.to_string()),
    )
    .await?;
    let written = write_tombstone(
        node,
        lease,
        membership.slot_index,
        bytes,
        &slot_kp.to_string(),
        &my_pseudonym_hex,
    )
    .await;
    crate::broadcast::dht_writes::release(node, lease).await;
    written?;
    info!(
        community = %membership.community_name,
        slot = membership.slot_index,
        "registry slot released"
    );
    Ok(())
}

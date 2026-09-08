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
    let gossip_payload = crate::payload::gossip::GossipPayload::Control(
        crate::payload::gossip::ControlPayload::MemberLeave {
            pseudonym_key: membership.pseudonym_key.clone(),
        },
    );
    let leave_payload_bytes =
        postcard::to_stdvec(&gossip_payload).map_err(|e| TransportError::SerializationFailed {
            reason: e.to_string(),
        })?;
    info!(community = %membership.community_name, "community left");
    Ok(LeaveResult {
        leave_payload_bytes,
    })
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
    let mut presence = rekindle_types::presence::MemberPresence {
        pseudonym_key: rekindle_types::id::PseudonymKey(pseudonym.verifying_key().to_bytes()),
        // "offline" so a reader that predates `departed` still drops us
        // from its roster rather than showing a ghost.
        status: "offline".into(),
        departed: true,
        last_heartbeat: rekindle_utils::timestamp_secs(),
        ..Default::default()
    };
    let sig = rekindle_secrets::derive::sign_with_pseudonym(&pseudonym, &presence.signing_bytes());
    presence.signature = sig.to_vec();
    let bytes = serde_json::to_vec(&presence).map_err(|e| TransportError::SerializationFailed {
        reason: e.to_string(),
    })?;

    crate::broadcast::dht_writes::open_writable(node, &membership.registry_key, slot_kp).await?;
    crate::broadcast::dht_writes::set(
        node,
        &membership.registry_key,
        membership.slot_index,
        bytes,
        None,
    )
    .await?;
    info!(
        community = %membership.community_name,
        slot = membership.slot_index,
        "registry slot released"
    );
    Ok(())
}

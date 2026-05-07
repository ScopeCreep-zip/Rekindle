//! Community leave — write leave entry to inbox, clear MEK cache.

use std::sync::Arc;

use parking_lot::RwLock;
use tracing::info;

use crate::crypto::mek::MekCache;
use crate::error::{TransportError, Result};
use crate::broadcast::node::TransportNode;
use crate::payload::dht_types::{PendingJoinEntry, PendingJoinStatus};

pub struct LeaveResult {
    pub leave_payload_bytes: Vec<u8>,
}

pub async fn leave_community(
    node: &TransportNode, membership: &crate::session::CommunityMembership,
    mek_cache: &Arc<RwLock<MekCache>>, signing_key_bytes: &[u8; 32],
) -> Result<LeaveResult> {
    info!(community = %membership.community_name, "leaving community via DHT");
    let dht = node.dht()?;

    let _ = crate::broadcast::dht_writes::open_readonly(node, &membership.governance_key).await;
    let metadata = dht.governance().read_metadata(&membership.governance_key).await?
        .ok_or_else(|| TransportError::Internal("governance metadata not found".into()))?;

    if !metadata.join_inbox_key.is_empty() && !metadata.join_inbox_keypair_hex.is_empty() {
        if let Ok(inbox_kp_bytes) = hex::decode(&metadata.join_inbox_keypair_hex) {
            if let Ok(inbox_kp) = crate::broadcast::node::deserialize_keypair(&inbox_kp_bytes) {
                let _ = crate::broadcast::dht_writes::open_writable(node, &metadata.join_inbox_key, inbox_kp).await;
                let pseudonym = crate::crypto::pseudonym::derive_community_pseudonym(signing_key_bytes, &membership.governance_key);
                let our_pseudonym_hex = hex::encode(pseudonym.verifying_key().to_bytes());
                let mut leave_entry = PendingJoinEntry {
                    requester_pseudonym_hex: our_pseudonym_hex.clone(), display_name: membership.display_name.clone(),
                    profile_dht_key: String::new(), invite_code_hash: None,
                    requested_at: rekindle_utils::timestamp_ms(),
                    status: PendingJoinStatus::Left { left_at: rekindle_utils::timestamp_ms() },
                    signature_hex: String::new(),
                };
                let content = leave_entry.signature_content();
                use ed25519_dalek::Signer;
                let sig = pseudonym.sign(&content);
                leave_entry.signature_hex = hex::encode(sig.to_bytes());
                let subkey = super::pseudonym_to_inbox_subkey(&our_pseudonym_hex);

                // Read-append-write
                let existing = match crate::broadcast::dht_writes::get(node, &metadata.join_inbox_key, subkey, true).await {
                    Ok(Some(data)) if !data.is_empty() && data != b"[]" => data,
                    _ => Vec::new(),
                };
                let mut inbox_entries: Vec<PendingJoinEntry> = if existing.is_empty() {
                    Vec::new()
                } else {
                    serde_json::from_slice::<Vec<PendingJoinEntry>>(&existing)
                        .or_else(|_| serde_json::from_slice::<PendingJoinEntry>(&existing).map(|e| vec![e]))
                        .unwrap_or_default()
                };
                inbox_entries.retain(|e| e.requester_pseudonym_hex != leave_entry.requester_pseudonym_hex);
                inbox_entries.push(leave_entry);
                let entry_bytes = serde_json::to_vec(&inbox_entries).map_err(|e| TransportError::SerializationFailed { reason: e.to_string() })?;
                let _ = crate::broadcast::dht_writes::set(node, &metadata.join_inbox_key, subkey, entry_bytes, None).await;
                info!(community = %membership.community_name, "leave entry written");
            }
        }
    }

    mek_cache.write().remove_community(&membership.governance_key);
    let gossip_payload = crate::payload::gossip::GossipPayload::Control(
        crate::payload::gossip::ControlPayload::MemberLeave { pseudonym_key: membership.pseudonym_key.clone() },
    );
    let leave_payload_bytes = postcard::to_stdvec(&gossip_payload).map_err(|e| TransportError::SerializationFailed { reason: e.to_string() })?;
    info!(community = %membership.community_name, "community left");
    Ok(LeaveResult { leave_payload_bytes })
}

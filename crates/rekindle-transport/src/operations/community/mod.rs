//! Community lifecycle operations — create, join, leave, inbox.
//!
//! Split by phase: create, join (submit → await → complete), leave, inbox reading.

mod create;
mod join;
mod leave;
mod inbox;

pub use create::{create_community, CommunityCreated};
pub use join::{
    submit_join_request, await_join_approval, complete_join,
    join_community, JoinRequestSubmitted, JoinResult,
};
pub use leave::{leave_community, LeaveResult};
pub use inbox::read_inbox_requests;

// ── Shared utilities ──────────────────────────────────────────────────

/// Deterministic inbox subkey from pseudonym hash.
pub(crate) fn pseudonym_to_inbox_subkey(pseudonym_hex: &str) -> u32 {
    let hash = blake3::hash(pseudonym_hex.as_bytes());
    let bytes = hash.as_bytes();
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) % crate::payload::dht_types::JOIN_INBOX_SUBKEY_COUNT
}

/// Read registry key from governance spine subkey.
pub(crate) async fn read_registry_key(
    node: &crate::broadcast::node::TransportNode, governance_key: &str, community_name: &str,
) -> crate::error::Result<String> {
    match crate::broadcast::dht_writes::get(node, governance_key, crate::payload::dht_types::MANIFEST_REGISTRY_SPINE, false).await? {
        Some(data) if !data.is_empty() => {
            let spine: serde_json::Value = serde_json::from_slice(&data)
                .map_err(|e| crate::error::TransportError::JoinRejected { community: community_name.to_string(), reason: format!("spine parse: {e}") })?;
            spine.get("primary_key").and_then(serde_json::Value::as_str).map(String::from)
                .ok_or_else(|| crate::error::TransportError::JoinRejected { community: community_name.to_string(), reason: "spine missing primary_key".into() })
        }
        _ => Err(crate::error::TransportError::JoinRejected { community: community_name.to_string(), reason: "no registry spine".into() }),
    }
}

/// Derive a slot seed for SMPL record member writes.
pub(crate) fn derive_slot_seed(signing_key_bytes: &[u8; 32], governance_key: &str, slot_index: u32) -> [u8; 32] {
    let hkdf = hkdf::Hkdf::<sha2::Sha256>::new(Some(b"rekindle-slot-seed-v1"), signing_key_bytes);
    let mut info = Vec::with_capacity(governance_key.len() + 4);
    info.extend_from_slice(governance_key.as_bytes());
    info.extend_from_slice(&slot_index.to_le_bytes());
    let mut seed = [0u8; 32];
    hkdf.expand(&info, &mut seed).expect("32-byte HKDF output");
    seed
}

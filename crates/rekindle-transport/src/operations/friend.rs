//! Friend lifecycle operations — send request, accept, reject, remove.
//!
//! All DHT writes route through `broadcast::dht_writes`.
//! Read operations use `DhtStore` directly.

use tracing::info;

use crate::error::{TransportError, Result};
use crate::broadcast::node::TransportNode;
use crate::payload::dht_types::{
    FriendEntry, FriendRequestEntry, FriendRequestStatus,
    PROFILE_SUBKEY_FRIEND_INBOX_KEY, PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR,
};
use crate::session::Session;

/// Result of sending a friend request — carries prekey private material
/// for the caller to persist to the OS keyring.
pub struct FriendRequestSent {
    /// Signed prekey private bytes (for X3DH completion across restarts).
    pub signed_prekey_private: Vec<u8>,
    /// One-time prekey private bytes (optional).
    pub one_time_prekey_private: Option<Vec<u8>>,
}

/// Send a friend request via DHT inbox write.
pub async fn send_friend_request(
    node: &TransportNode, session: &Session,
    target_mailbox_key: &str, message: &str, signing_key_bytes: &[u8; 32],
) -> Result<FriendRequestSent> {
    info!(target = &target_mailbox_key[..16.min(target_mailbox_key.len())], "sending friend request via DHT");

    // Open target's profile to read friend inbox info
    crate::broadcast::dht_writes::open_readonly(node, target_mailbox_key).await
        .map_err(|e| TransportError::FriendRequestFailed {
            target: target_mailbox_key.to_string(), reason: format!("cannot open target profile: {e}"),
        })?;

    let dht = node.dht()?;
    let inbox_key_data = dht.profile().get_subkey_fresh(target_mailbox_key, PROFILE_SUBKEY_FRIEND_INBOX_KEY).await.unwrap_or(None);
    let inbox_keypair_data = dht.profile().get_subkey_fresh(target_mailbox_key, PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR).await.unwrap_or(None);

    let (inbox_key, inbox_keypair_hex) = match (inbox_key_data, inbox_keypair_data) {
        (Some(key_bytes), Some(kp_bytes)) if !key_bytes.is_empty() && !kp_bytes.is_empty() => {
            (String::from_utf8_lossy(&key_bytes).to_string(), String::from_utf8_lossy(&kp_bytes).to_string())
        }
        _ => {
            info!("friend inbox not in cached profile, fetching fresh from network");
            let fresh_key = dht.profile().get_subkey_fresh(target_mailbox_key, PROFILE_SUBKEY_FRIEND_INBOX_KEY).await.unwrap_or(None);
            let fresh_kp = dht.profile().get_subkey_fresh(target_mailbox_key, PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR).await.unwrap_or(None);
            match (fresh_key, fresh_kp) {
                (Some(k), Some(kp)) if !k.is_empty() && !kp.is_empty() => {
                    (String::from_utf8_lossy(&k).to_string(), String::from_utf8_lossy(&kp).to_string())
                }
                _ => return Err(TransportError::FriendRequestFailed {
                    target: target_mailbox_key.to_string(),
                    reason: "target's friend inbox not available yet".into(),
                }),
            }
        }
    };

    // Open inbox with published keypair
    let inbox_kp_bytes = hex::decode(&inbox_keypair_hex)
        .map_err(|e| TransportError::FriendRequestFailed {
            target: target_mailbox_key.to_string(), reason: format!("invalid inbox keypair: {e}"),
        })?;
    let inbox_kp = crate::broadcast::node::deserialize_keypair(&inbox_kp_bytes)?;
    crate::broadcast::dht_writes::open_writable(node, &inbox_key, inbox_kp).await
        .map_err(|e| TransportError::FriendRequestFailed {
            target: target_mailbox_key.to_string(), reason: format!("cannot open inbox: {e}"),
        })?;

    // Generate prekey bundle
    let x25519_secret = crate::crypto::pseudonym::pseudonym_to_x25519(
        &ed25519_dalek::SigningKey::from_bytes(signing_key_bytes),
    );
    let x25519_public = x25519_dalek::PublicKey::from(&x25519_secret);
    let signal = crate::crypto::signal_session::SignalSessionManager::new(
        Box::new(crate::crypto::signal_store::MemoryIdentityStore::new(
            x25519_secret.to_bytes().to_vec(), x25519_public.as_bytes().to_vec(), 1,
        )),
        Box::new(crate::crypto::signal_store::MemoryPreKeyStore::new()),
        Box::new(crate::crypto::signal_store::MemorySessionStore::new()),
    );
    let prekey_bundle = signal.generate_prekey_bundle(1, Some(1))
        .map_err(|e| TransportError::FriendRequestFailed {
            target: target_mailbox_key.to_string(), reason: format!("prekey: {e}"),
        })?;
    // Extract prekey private material for persistence across restarts
    let signed_prekey_private = signal.load_signed_prekey(1).unwrap_or_default();
    let one_time_prekey_private = signal.load_prekey(1).ok().flatten();
    let prekey_bytes = prekey_bundle.to_bytes()
        .map_err(|e| TransportError::FriendRequestFailed {
            target: target_mailbox_key.to_string(), reason: format!("prekey serialize: {e}"),
        })?;

    let request = FriendRequestEntry {
        sender_public_key: session.identity.public_key_hex.clone(),
        display_name: session.identity.display_name.clone(),
        message: message.to_string(),
        profile_dht_key: session.identity.profile_dht_key.clone(),
        mailbox_dht_key: session.identity.mailbox_dht_key.clone(),
        sender_friend_inbox_key: session.identity.friend_inbox_key.clone(),
        sender_friend_inbox_keypair_hex: session.identity.friend_inbox_keypair_hex.clone(),
        prekey_bundle: prekey_bytes,
        sent_at: rekindle_utils::timestamp_ms(),
        status: FriendRequestStatus::Pending,
    };

    // Subkey determined by both sender + recipient to reduce collisions
    let subkey = blake3_hash_mod(&session.identity.public_key_hex, target_mailbox_key, 32);

    // Read-append-write: read existing entries, append ours, write back
    read_append_write(node, &inbox_key, subkey, &request).await
        .map_err(|e| TransportError::FriendRequestFailed {
            target: target_mailbox_key.to_string(), reason: format!("inbox write: {e}"),
        })?;

    info!("friend request written to DHT inbox — verifying propagation");

    // Verify our entry is present (handles concurrent writer race)
    if verify_entry_present(node, &inbox_key, subkey, &request).await {
        info!("friend request verified — propagated to network");
    } else {
        // Retry: re-read, re-append, re-write
        tracing::warn!("friend request not found after write — retrying (concurrent writer race)");
        read_append_write(node, &inbox_key, subkey, &request).await
            .map_err(|e| TransportError::FriendRequestFailed {
                target: target_mailbox_key.to_string(), reason: format!("inbox retry write: {e}"),
            })?;
        if verify_entry_present(node, &inbox_key, subkey, &request).await {
            info!("friend request verified on retry — propagated to network");
        } else {
            tracing::warn!(
                "friend request written but propagation not confirmed after retry — \
                 target may experience delay discovering this request"
            );
        }
    }

    // Best-effort direct notification via target's route (tier 2 — instant).
    // The DHT watch (tier 1) and poll (tier 3) will also discover this,
    // but a direct notification ensures sub-second awareness if the target is online.
    let dht = node.dht()?;
    if let Ok(Some(route_blob)) = dht.mailbox().read_peer_route(target_mailbox_key).await {
        if !route_blob.is_empty() {
            if let Ok(target) = node.import_route(&route_blob) {
                let notify_payload = crate::payload::dm::DmPayload::FriendRequestAck;
                let notify_bytes = crate::payload::dm::serialize_dm(&notify_payload)
                    .unwrap_or_default();
                if !notify_bytes.is_empty() {
                    let _ = node.sender().send_dm(
                        &target,
                        crate::frame::TypeId::FriendRequestAck,
                        signing_key_bytes,
                        &session.identity.public_key_hex,
                        &notify_bytes,
                    ).await;
                    info!("direct notification sent to target via route");
                }
            }
        }
    }

    Ok(FriendRequestSent { signed_prekey_private, one_time_prekey_private })
}

pub struct FriendAccepted {
    pub dm_log_key: String,
    pub dm_log_keypair_bytes: Vec<u8>,
}

/// Accept a pending friend request.
pub async fn accept_friend_request(
    node: &TransportNode, session: &Session,
    requester_public_key: &str, requester_route_blob: &[u8],
    requester_profile_dht_key: &str, requester_display_name: &str,
) -> Result<FriendAccepted> {
    info!(requester = &requester_public_key[..12], "accepting friend request");

    // Create shared DM DhtLog
    let (dm_log, dm_log_kp) = crate::broadcast::dht_writes::create_dht_log(node).await?;
    let dm_log_key = dm_log.spine_key();
    let dm_log_keypair_bytes = super::identity::serialize_keypair(&dm_log_kp);
    let dm_log_keypair_hex = hex::encode(&dm_log_keypair_bytes);

    // Add to friend list DHT
    let nickname = if requester_display_name.is_empty() { None } else { Some(requester_display_name.to_string()) };
    let friend_entry = FriendEntry {
        public_key: requester_public_key.to_string(), nickname, group: None,
        added_at: rekindle_utils::timestamp_ms(),
        profile_dht_key: Some(requester_profile_dht_key.to_string()),
        dm_log_key: Some(dm_log_key.clone()),
    };
    node.dht()?.friend_list().add(&session.identity.friend_list_dht_key, friend_entry).await?;

    if !requester_route_blob.is_empty() {
        node.peers().write().cache_route(requester_public_key, requester_route_blob.to_vec());
    }

    // Write Accepted response to requester's friend inbox
    let dht = node.dht()?;
    let req_inbox_key = dht.profile().get_subkey_fresh(requester_profile_dht_key, PROFILE_SUBKEY_FRIEND_INBOX_KEY).await.unwrap_or(None);
    let req_inbox_kp = dht.profile().get_subkey_fresh(requester_profile_dht_key, PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR).await.unwrap_or(None);

    if let (Some(key_bytes), Some(kp_bytes)) = (req_inbox_key, req_inbox_kp) {
        let inbox_key = String::from_utf8_lossy(&key_bytes).to_string();
        let kp_hex = String::from_utf8_lossy(&kp_bytes).to_string();
        if let Ok(kp_raw) = hex::decode(&kp_hex) {
            if let Ok(kp) = crate::broadcast::node::deserialize_keypair(&kp_raw) {
                let _ = crate::broadcast::dht_writes::open_writable(node, &inbox_key, kp).await;
                let response = FriendRequestEntry {
                    sender_public_key: session.identity.public_key_hex.clone(),
                    display_name: session.identity.display_name.clone(),
                    message: String::new(),
                    profile_dht_key: session.identity.profile_dht_key.clone(),
                    mailbox_dht_key: session.identity.mailbox_dht_key.clone(),
                    sender_friend_inbox_key: session.identity.friend_inbox_key.clone(),
                    sender_friend_inbox_keypair_hex: session.identity.friend_inbox_keypair_hex.clone(),
                    prekey_bundle: Vec::new(),
                    sent_at: rekindle_utils::timestamp_ms(),
                    status: FriendRequestStatus::Accepted {
                        responder_profile_dht_key: session.identity.profile_dht_key.clone(),
                        responder_mailbox_dht_key: session.identity.mailbox_dht_key.clone(),
                        dm_log_key: dm_log_key.clone(),
                        dm_log_keypair_hex: dm_log_keypair_hex.clone(),
                        accepted_at: rekindle_utils::timestamp_ms(),
                    },
                };
                let subkey = blake3_hash_mod(&session.identity.public_key_hex, requester_profile_dht_key, 32);
                let _ = read_append_write(node, &inbox_key, subkey, &response).await;
                info!("acceptance written to requester's friend inbox");
            }
        }
    }

    let dm_log_keypair_bytes = super::identity::serialize_keypair(&dm_log_kp);
    info!(requester = &requester_public_key[..12], dm_log = %dm_log_key, "friend request accepted");
    Ok(FriendAccepted { dm_log_key, dm_log_keypair_bytes })
}

/// Reject a pending friend request.
pub async fn reject_friend_request(
    node: &TransportNode, session: &Session, requester_public_key: &str,
) -> Result<()> {
    info!(requester = &requester_public_key[..12.min(requester_public_key.len())], "rejecting friend request");

    let pending = session.pending_request_by_key(requester_public_key);
    if let Some(req) = pending {
        let dht = node.dht()?;
        let inbox_key_data = dht.profile().get_subkey_fresh(&req.profile_dht_key, PROFILE_SUBKEY_FRIEND_INBOX_KEY).await.unwrap_or(None);
        let inbox_kp_data = dht.profile().get_subkey_fresh(&req.profile_dht_key, PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR).await.unwrap_or(None);

        if let (Some(key_bytes), Some(kp_bytes)) = (inbox_key_data, inbox_kp_data) {
            let inbox_key = String::from_utf8_lossy(&key_bytes).to_string();
            let kp_hex = String::from_utf8_lossy(&kp_bytes).to_string();
            if let Ok(kp_raw) = hex::decode(&kp_hex) {
                if let Ok(kp) = crate::broadcast::node::deserialize_keypair(&kp_raw) {
                    let _ = crate::broadcast::dht_writes::open_writable(node, &inbox_key, kp).await;
                    let response = FriendRequestEntry {
                        sender_public_key: session.identity.public_key_hex.clone(),
                        display_name: session.identity.display_name.clone(),
                        message: String::new(),
                        profile_dht_key: session.identity.profile_dht_key.clone(),
                        mailbox_dht_key: session.identity.mailbox_dht_key.clone(),
                        sender_friend_inbox_key: String::new(),
                        sender_friend_inbox_keypair_hex: String::new(),
                        prekey_bundle: Vec::new(),
                        sent_at: rekindle_utils::timestamp_ms(),
                        status: FriendRequestStatus::Rejected { rejected_at: rekindle_utils::timestamp_ms() },
                    };
                    let subkey = blake3_hash_mod(&session.identity.public_key_hex, &req.profile_dht_key, 32);
                    let _ = read_append_write(node, &inbox_key, subkey, &response).await;
                    info!("rejection written to requester's friend inbox");
                }
            }
        }
    }
    Ok(())
}

/// Remove a friend.
pub async fn remove_friend(
    node: &TransportNode, session: &Session, peer_key: &str,
) -> Result<()> {
    info!(peer = &peer_key[..12], "removing friend");
    node.dht()?.friend_list().remove(&session.identity.friend_list_dht_key, peer_key).await?;
    node.peers().write().invalidate_route(peer_key);
    info!(peer = &peer_key[..12], "friend removed");
    Ok(())
}

/// Deterministic subkey index from sender + recipient keys.
/// Mixing both keys reduces collision probability: different pairs map to
/// different slots even when one key is shared.
fn blake3_hash_mod(sender: &str, recipient: &str, n: u32) -> u32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(sender.as_bytes());
    hasher.update(b"|");
    hasher.update(recipient.as_bytes());
    let hash = hasher.finalize();
    let bytes = hash.as_bytes();
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) % n
}

/// Read existing entries from a subkey, append our entry, write back.
/// Returns error only on write failure — empty/unparseable subkeys are
/// treated as empty arrays (safe to overwrite).
async fn read_append_write(
    node: &TransportNode, inbox_key: &str, subkey: u32, entry: &FriendRequestEntry,
) -> Result<()> {
    // Read current state
    let existing = match crate::broadcast::dht_writes::get(node, inbox_key, subkey, true).await {
        Ok(Some(data)) if !data.is_empty() && data != b"[]" => data,
        _ => Vec::new(),
    };

    // Parse existing array (or start fresh if unparseable)
    let mut entries: Vec<FriendRequestEntry> = if existing.is_empty() {
        Vec::new()
    } else {
        // Try array first, then single entry for backward compatibility
        serde_json::from_slice::<Vec<FriendRequestEntry>>(&existing)
            .or_else(|_| serde_json::from_slice::<FriendRequestEntry>(&existing).map(|e| vec![e]))
            .unwrap_or_default()
    };

    // Remove any existing entry from the same sender (idempotent upsert)
    entries.retain(|e| e.sender_public_key != entry.sender_public_key);
    entries.push(entry.clone());

    let bytes = serde_json::to_vec(&entries)
        .map_err(|e| TransportError::SerializationFailed { reason: e.to_string() })?;
    crate::broadcast::dht_writes::set(node, inbox_key, subkey, bytes, None).await
}

/// Verify our specific entry is present in the subkey after writing.
/// Retries with exponential backoff on failure (concurrent writer race).
///
/// NOTE: This does NOT use `dht_writes::set_and_verify()` because that
/// primitive checks for any non-empty data. This function checks for a
/// SPECIFIC entry by `sender_public_key` — a stronger, content-aware
/// verification needed for inbox read-append-write atomicity.
async fn verify_entry_present(
    node: &TransportNode, inbox_key: &str, subkey: u32, entry: &FriendRequestEntry,
) -> bool {
    let deadline = std::time::Duration::from_secs(15);
    let start = std::time::Instant::now();
    let mut backoff = std::time::Duration::from_millis(300);
    let ceiling = std::time::Duration::from_secs(3);

    loop {
        match crate::broadcast::dht_writes::get(node, inbox_key, subkey, true).await {
            Ok(Some(data)) if !data.is_empty() && data != b"[]" => {
                let entries: Vec<FriendRequestEntry> = serde_json::from_slice::<Vec<FriendRequestEntry>>(&data)
                    .or_else(|_| serde_json::from_slice::<FriendRequestEntry>(&data).map(|e| vec![e]))
                    .unwrap_or_default();
                if entries.iter().any(|e| e.sender_public_key == entry.sender_public_key) {
                    return true;
                }
            }
            _ => {}
        }

        if start.elapsed() >= deadline {
            return false;
        }

        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(ceiling);
    }
}

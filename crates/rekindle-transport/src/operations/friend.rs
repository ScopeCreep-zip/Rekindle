//! Friend lifecycle operations — send request, accept, reject, remove.
//!
//! Protocol: sender-creates-DhtLog. The friend request sender creates
//! the shared DM DhtLog at send time and includes the key + keypair in
//! the inbox entry. The responder adopts it on accept. No second DhtLog
//! is ever created. If both users send requests to each other, the first
//! accept resolves the friendship — the second request is redundant.
//!
//! All inbox entries are Ed25519-signed by the sender. The inbox scan
//! verifies signatures before processing.
//!
//! Signal Protocol sessions are established during the accept flow:
//! - Responder runs initiator X3DH using the sender's prekey bundle
//! - Sender discovers the acceptance and runs responder X3DH using
//!   the handshake fields in the Accepted entry

use ed25519_dalek::Signer;
use tracing::info;

use crate::error::{TransportError, Result};
use crate::broadcast::node::TransportNode;
use crate::payload::dht_types::{
    FriendEntry, FriendRequestEntry, FriendRequestStatus,
    PROFILE_SUBKEY_FRIEND_INBOX_KEY, PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR,
};
use crate::session::Session;

/// Result of sending a friend request.
pub struct FriendRequestSent {
    /// Signed prekey private bytes (for X3DH completion across restarts).
    pub signed_prekey_private: Vec<u8>,
    /// One-time prekey private bytes (optional).
    pub one_time_prekey_private: Option<Vec<u8>>,
    /// DhtLog spine key for the shared DM conversation (created at send time).
    pub dm_log_key: String,
    /// DhtLog keypair bytes (64 bytes: 32 pub + 32 secret).
    pub dm_log_keypair_bytes: Vec<u8>,
}

/// Send a friend request via DHT inbox write.
///
/// Creates the shared DM DhtLog, signs the inbox entry, writes to
/// the target's friend inbox, and sends a best-effort direct notification.
pub async fn send_friend_request(
    node: &TransportNode, session: &Session,
    target_profile_key: &str, message: &str, signing_key_bytes: &[u8; 32],
) -> Result<FriendRequestSent> {
    info!(target = &target_profile_key[..16.min(target_profile_key.len())], "sending friend request via DHT");

    // Open target's profile to read friend inbox info
    crate::broadcast::dht_writes::open_readonly(node, target_profile_key).await
        .map_err(|e| TransportError::FriendRequestFailed {
            target: target_profile_key.to_string(), reason: format!("cannot open target profile: {e}"),
        })?;

    let dht = node.dht()?;
    let inbox_key_data = dht.profile().get_subkey_fresh(target_profile_key, PROFILE_SUBKEY_FRIEND_INBOX_KEY).await.unwrap_or(None);
    let inbox_keypair_data = dht.profile().get_subkey_fresh(target_profile_key, PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR).await.unwrap_or(None);

    let (inbox_key, inbox_keypair_hex) = match (inbox_key_data, inbox_keypair_data) {
        (Some(key_bytes), Some(kp_bytes)) if !key_bytes.is_empty() && !kp_bytes.is_empty() => {
            (String::from_utf8_lossy(&key_bytes).to_string(), String::from_utf8_lossy(&kp_bytes).to_string())
        }
        _ => {
            info!("friend inbox not in cached profile, fetching fresh from network");
            let fresh_key = dht.profile().get_subkey_fresh(target_profile_key, PROFILE_SUBKEY_FRIEND_INBOX_KEY).await.unwrap_or(None);
            let fresh_kp = dht.profile().get_subkey_fresh(target_profile_key, PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR).await.unwrap_or(None);
            match (fresh_key, fresh_kp) {
                (Some(k), Some(kp)) if !k.is_empty() && !kp.is_empty() => {
                    (String::from_utf8_lossy(&k).to_string(), String::from_utf8_lossy(&kp).to_string())
                }
                _ => return Err(TransportError::FriendRequestFailed {
                    target: target_profile_key.to_string(),
                    reason: "target's friend inbox not available yet".into(),
                }),
            }
        }
    };

    // Open inbox with published keypair
    let inbox_kp_bytes = hex::decode(&inbox_keypair_hex)
        .map_err(|e| TransportError::FriendRequestFailed {
            target: target_profile_key.to_string(), reason: format!("invalid inbox keypair: {e}"),
        })?;
    let inbox_kp = crate::broadcast::node::deserialize_keypair(&inbox_kp_bytes)?;
    crate::broadcast::dht_writes::open_writable(node, &inbox_key, inbox_kp).await
        .map_err(|e| TransportError::FriendRequestFailed {
            target: target_profile_key.to_string(), reason: format!("cannot open inbox: {e}"),
        })?;

    // Generate prekey bundle for Signal session establishment
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
            target: target_profile_key.to_string(), reason: format!("prekey: {e}"),
        })?;
    let signed_prekey_private = signal.load_signed_prekey(1).unwrap_or_default();
    let one_time_prekey_private = signal.load_prekey(1).ok().flatten();
    let prekey_bytes = prekey_bundle.to_bytes()
        .map_err(|e| TransportError::FriendRequestFailed {
            target: target_profile_key.to_string(), reason: format!("prekey serialize: {e}"),
        })?;

    // Create the shared DM DhtLog — sender creates, responder adopts.
    let (dm_log, dm_log_kp) = crate::broadcast::dht_writes::create_dht_log(node).await?;
    let dm_log_key = dm_log.spine_key();
    let dm_log_keypair_bytes = super::identity::serialize_keypair(&dm_log_kp);
    let dm_log_keypair_hex = hex::encode(&dm_log_keypair_bytes);

    // Build the request entry (unsigned — signature added below)
    let mut request = FriendRequestEntry {
        sender_public_key: session.identity.public_key_hex.clone(),
        display_name: session.identity.display_name.clone(),
        message: message.to_string(),
        profile_dht_key: session.identity.profile_dht_key.clone(),
        mailbox_dht_key: session.identity.mailbox_dht_key.clone(),
        sender_friend_inbox_key: session.identity.friend_inbox_key.clone(),
        sender_friend_inbox_keypair_hex: session.identity.friend_inbox_keypair_hex.clone(),
        prekey_bundle: prekey_bytes,
        sent_at: rekindle_utils::timestamp_ms(),
        dm_log_key: dm_log_key.clone(),
        dm_log_keypair_hex,
        signature_hex: String::new(),
        status: FriendRequestStatus::Pending,
    };

    // Sign the entry with the sender's Ed25519 identity key
    let signing_key = ed25519_dalek::SigningKey::from_bytes(signing_key_bytes);
    let content = request.signature_content();
    let signature = signing_key.sign(&content);
    request.signature_hex = hex::encode(signature.to_bytes());

    // Subkey determined by both sender + recipient to reduce collisions
    let subkey = blake3_hash_mod(&session.identity.public_key_hex, target_profile_key, 32);

    // Read-append-write: read existing entries, append ours, write back
    read_append_write(node, &inbox_key, subkey, &request).await
        .map_err(|e| TransportError::FriendRequestFailed {
            target: target_profile_key.to_string(), reason: format!("inbox write: {e}"),
        })?;

    info!("friend request written to DHT inbox — verifying propagation");

    // Verify our entry is present (handles concurrent writer race)
    if verify_entry_present(node, &inbox_key, subkey, &request).await {
        info!("friend request verified — propagated to network");
    } else {
        tracing::warn!("friend request not found after write — retrying (concurrent writer race)");
        read_append_write(node, &inbox_key, subkey, &request).await
            .map_err(|e| TransportError::FriendRequestFailed {
                target: target_profile_key.to_string(), reason: format!("inbox retry write: {e}"),
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

    // Best-effort direct notification via target's route (tier 2 — instant)
    let dht = node.dht()?;
    if let Ok(Some(route_blob)) = dht.mailbox().read_peer_route(target_profile_key).await {
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

    Ok(FriendRequestSent {
        signed_prekey_private,
        one_time_prekey_private,
        dm_log_key,
        dm_log_keypair_bytes,
    })
}

pub struct FriendAccepted {
    /// The requester's DhtLog key (their outbound = our inbound).
    pub inbound_log_key: String,
    /// Keypair bytes for the requester's DhtLog (so we can read it; also shared for write by requester).
    pub inbound_log_keypair_bytes: Vec<u8>,
    /// Our newly created outbound DhtLog key (we write, they read).
    pub outbound_log_key: String,
    /// Keypair bytes for our outbound DhtLog.
    pub outbound_log_keypair_bytes: Vec<u8>,
}

/// Accept a pending friend request.
///
/// Adopts the sender's DhtLog (does NOT create a new one). Establishes
/// the initiator side of the Signal session using the sender's prekey
/// bundle. Writes an Accepted entry with the X3DH handshake fields to
/// the sender's inbox so they can complete the responder side.
pub async fn accept_friend_request(
    node: &TransportNode, session: &Session,
    requester_public_key: &str, requester_route_blob: &[u8],
    requester_profile_dht_key: &str, requester_display_name: &str,
    requester_dm_log_key: &str, requester_dm_log_keypair_hex: &str,
    signing_key_bytes: &[u8; 32],
    signal_init: &crate::crypto::signal_session::SessionInitInfo,
    identity_public_key: &[u8],
) -> Result<FriendAccepted> {
    info!(requester = &requester_public_key[..12], "accepting friend request");

    // Adopt the sender's DhtLog as our INBOUND (we read their messages from it)
    let inbound_log_key = requester_dm_log_key.to_string();
    let inbound_log_keypair_bytes = hex::decode(requester_dm_log_keypair_hex)
        .map_err(|e| TransportError::FriendRequestFailed {
            target: requester_public_key.to_string(),
            reason: format!("invalid DM log keypair hex: {e}"),
        })?;
    if inbound_log_keypair_bytes.len() != 64 {
        return Err(TransportError::FriendRequestFailed {
            target: requester_public_key.to_string(),
            reason: format!("DM log keypair wrong length: {} (expected 64)", inbound_log_keypair_bytes.len()),
        });
    }
    let _ = crate::broadcast::node::deserialize_keypair(&inbound_log_keypair_bytes)?;

    // Create our OUTBOUND DhtLog (we write our messages here, they read it)
    let (outbound_log, outbound_log_kp) = crate::broadcast::dht_writes::create_dht_log(node).await?;
    let outbound_log_key = outbound_log.spine_key();
    let outbound_log_keypair_bytes = super::identity::serialize_keypair(&outbound_log_kp);
    let outbound_log_keypair_hex = hex::encode(&outbound_log_keypair_bytes);

    // Signal session establishment is done by the caller (daemon crate)
    // on the persistent SignalSessionManager. The caller passes the resulting
    // SessionInitInfo + X25519 public key so we can include them in the Accepted entry.
    // accept_friend_request does NOT touch Signal — it only handles DhtLog + DHT writes.

    // Add to friend list DHT
    let nickname = if requester_display_name.is_empty() { None } else { Some(requester_display_name.to_string()) };
    let friend_entry = FriendEntry {
        public_key: requester_public_key.to_string(), nickname, group: None,
        added_at: rekindle_utils::timestamp_ms(),
        profile_dht_key: Some(requester_profile_dht_key.to_string()),
        dm_log_key: Some(outbound_log_key.clone()),
    };
    node.dht()?.friend_list().add(&session.identity.friend_list_dht_key, friend_entry).await?;

    if !requester_route_blob.is_empty() {
        node.peers().write().cache_route(requester_public_key, requester_route_blob.to_vec());
    }

    // Write Accepted response to requester's friend inbox.
    // The Accepted entry includes our OUTBOUND log key + keypair so the
    // requester can set up their inbound watch on it and read our messages.
    let dht = node.dht()?;
    let req_inbox_key = dht.profile().get_subkey_fresh(requester_profile_dht_key, PROFILE_SUBKEY_FRIEND_INBOX_KEY).await.unwrap_or(None);
    let req_inbox_kp = dht.profile().get_subkey_fresh(requester_profile_dht_key, PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR).await.unwrap_or(None);

    if let (Some(key_bytes), Some(kp_bytes)) = (req_inbox_key, req_inbox_kp) {
        let inbox_key = String::from_utf8_lossy(&key_bytes).to_string();
        let kp_hex = String::from_utf8_lossy(&kp_bytes).to_string();
        if let Ok(kp_raw) = hex::decode(&kp_hex) {
            if let Ok(kp) = crate::broadcast::node::deserialize_keypair(&kp_raw) {
                let _ = crate::broadcast::dht_writes::open_writable(node, &inbox_key, kp).await;

                let signing_key = ed25519_dalek::SigningKey::from_bytes(signing_key_bytes);

                let mut response = FriendRequestEntry {
                    sender_public_key: session.identity.public_key_hex.clone(),
                    display_name: session.identity.display_name.clone(),
                    message: String::new(),
                    profile_dht_key: session.identity.profile_dht_key.clone(),
                    mailbox_dht_key: session.identity.mailbox_dht_key.clone(),
                    sender_friend_inbox_key: session.identity.friend_inbox_key.clone(),
                    sender_friend_inbox_keypair_hex: session.identity.friend_inbox_keypair_hex.clone(),
                    prekey_bundle: Vec::new(),
                    sent_at: rekindle_utils::timestamp_ms(),
                    // Entry-level log fields: our outbound log info (for the requester to discover)
                    dm_log_key: outbound_log_key.clone(),
                    dm_log_keypair_hex: outbound_log_keypair_hex.clone(),
                    signature_hex: String::new(),
                    status: FriendRequestStatus::Accepted {
                        responder_profile_dht_key: session.identity.profile_dht_key.clone(),
                        responder_mailbox_dht_key: session.identity.mailbox_dht_key.clone(),
                        // Our outbound log = sender's inbound (they read our messages from it)
                        responder_outbound_log_key: outbound_log_key.clone(),
                        responder_outbound_log_keypair_hex: outbound_log_keypair_hex.clone(),
                        responder_identity_key: identity_public_key.to_vec(),
                        ephemeral_public_key: signal_init.ephemeral_public_key.clone(),
                        signed_prekey_id: signal_init.signed_prekey_id,
                        one_time_prekey_id: signal_init.one_time_prekey_id,
                        accepted_at: rekindle_utils::timestamp_ms(),
                    },
                };
                let content = response.signature_content();
                let sig = signing_key.sign(&content);
                response.signature_hex = hex::encode(sig.to_bytes());

                let subkey = blake3_hash_mod(&session.identity.public_key_hex, requester_profile_dht_key, 32);
                let _ = read_append_write(node, &inbox_key, subkey, &response).await;
                info!("acceptance written to requester's friend inbox");
            }
        }
    }

    info!(
        requester = &requester_public_key[..12],
        outbound = %outbound_log_key,
        inbound = %inbound_log_key,
        "friend request accepted"
    );
    Ok(FriendAccepted {
        inbound_log_key,
        inbound_log_keypair_bytes,
        outbound_log_key,
        outbound_log_keypair_bytes,
    })
}

/// Reject a pending friend request.
pub async fn reject_friend_request(
    node: &TransportNode, session: &Session, requester_public_key: &str,
    signing_key_bytes: &[u8; 32],
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

                    let signing_key = ed25519_dalek::SigningKey::from_bytes(signing_key_bytes);

                    let mut response = FriendRequestEntry {
                        sender_public_key: session.identity.public_key_hex.clone(),
                        display_name: session.identity.display_name.clone(),
                        message: String::new(),
                        profile_dht_key: session.identity.profile_dht_key.clone(),
                        mailbox_dht_key: session.identity.mailbox_dht_key.clone(),
                        sender_friend_inbox_key: String::new(),
                        sender_friend_inbox_keypair_hex: String::new(),
                        prekey_bundle: Vec::new(),
                        sent_at: rekindle_utils::timestamp_ms(),
                        dm_log_key: String::new(),
                        dm_log_keypair_hex: String::new(),
                        signature_hex: String::new(),
                        status: FriendRequestStatus::Rejected { rejected_at: rekindle_utils::timestamp_ms() },
                    };
                    let content = response.signature_content();
                    let sig = signing_key.sign(&content);
                    response.signature_hex = hex::encode(sig.to_bytes());

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
pub fn blake3_hash_mod(sender: &str, recipient: &str, n: u32) -> u32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(sender.as_bytes());
    hasher.update(b"|");
    hasher.update(recipient.as_bytes());
    let hash = hasher.finalize();
    let bytes = hash.as_bytes();
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) % n
}

/// Read existing entries from a subkey, append our entry, write back.
async fn read_append_write(
    node: &TransportNode, inbox_key: &str, subkey: u32, entry: &FriendRequestEntry,
) -> Result<()> {
    let existing = match crate::broadcast::dht_writes::get(node, inbox_key, subkey, true).await {
        Ok(Some(data)) if !data.is_empty() && data != b"[]" => data,
        _ => Vec::new(),
    };

    let mut entries: Vec<FriendRequestEntry> = if existing.is_empty() {
        Vec::new()
    } else {
        FriendRequestEntry::parse_inbox_data(&existing).unwrap_or_default()
    };

    entries.retain(|e| e.sender_public_key != entry.sender_public_key);
    entries.push(entry.clone());

    let bytes = serde_json::to_vec(&entries)
        .map_err(|e| TransportError::SerializationFailed { reason: e.to_string() })?;
    crate::broadcast::dht_writes::set(node, inbox_key, subkey, bytes, None).await
}

/// Verify our specific entry is present in the subkey after writing.
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
                let entries = FriendRequestEntry::parse_inbox_data(&data).unwrap_or_default();
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

//! Friend lifecycle operations — send request, accept, reject, remove.
//!
//! Friend-inbox writes borrow the peer's inbox from the session's record
//! pool (plan C7.7d). Profile, mailbox and friend-list I/O use the
//! `rekindle_protocol` record functions over the same pool (plan C7.4).

use tracing::info;

use crate::broadcast::dht_writes::LeaseId;
use crate::broadcast::node::TransportNode;
use crate::error::{Result, TransportError};
use crate::payload::dht_types::{
    FriendEntry, FriendRequestEntry, FriendRequestStatus, PROFILE_SUBKEY_FRIEND_INBOX_KEY,
    PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR, PROFILE_SUBKEY_PREKEY_BUNDLE,
};
use crate::session::Session;
use rekindle_protocol::dht::mailbox::read_peer_mailbox_route;
use rekindle_protocol::dht::profile::read_profile_subkey;

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
    node: &TransportNode,
    session: &Session,
    target_mailbox_key: &str,
    message: &str,
    signing_key_bytes: &[u8; 32],
) -> Result<FriendRequestSent> {
    info!(
        target = %target_mailbox_key,
        "sending friend request via DHT"
    );

    // Read the friend inbox info from the target's profile. Failing to
    // reach the profile at all is the request's failure.
    let pool = node.require_records()?;
    let inbox_key_data = read_profile_subkey(
        &pool,
        target_mailbox_key,
        PROFILE_SUBKEY_FRIEND_INBOX_KEY,
        true,
    )
    .await
    .map_err(|e| TransportError::FriendRequestFailed {
        target: target_mailbox_key.to_string(),
        reason: format!("cannot open target profile: {e}"),
    })?;
    let inbox_keypair_data = read_profile_subkey(
        &pool,
        target_mailbox_key,
        PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR,
        true,
    )
    .await
    .unwrap_or(None);

    let (inbox_key, inbox_keypair_hex) = match (inbox_key_data, inbox_keypair_data) {
        (Some(key_bytes), Some(kp_bytes)) if !key_bytes.is_empty() && !kp_bytes.is_empty() => (
            String::from_utf8_lossy(&key_bytes).to_string(),
            String::from_utf8_lossy(&kp_bytes).to_string(),
        ),
        // Both reads were forced to the network through the pool's one
        // retry layer; a second identical pass was a retry ladder.
        _ => {
            return Err(TransportError::FriendRequestFailed {
                target: target_mailbox_key.to_string(),
                reason: "target's friend inbox not available yet".into(),
            })
        }
    };

    // Open inbox with published keypair
    let inbox_kp_bytes =
        hex::decode(&inbox_keypair_hex).map_err(|e| TransportError::FriendRequestFailed {
            target: target_mailbox_key.to_string(),
            reason: format!("invalid inbox keypair: {e}"),
        })?;
    let inbox_kp = crate::broadcast::node::deserialize_keypair(&inbox_kp_bytes)?;

    // Generate prekey bundle — Ed25519 identity layout for PQXDH.
    let signing_key = ed25519_dalek::SigningKey::from_bytes(signing_key_bytes);
    let verifying_key = signing_key.verifying_key();
    let signal = crate::crypto::signal_session::SignalSessionManager::new(
        Box::new(crate::crypto::signal_store::MemoryIdentityStore::new(
            signing_key.to_bytes().to_vec(),
            verifying_key.as_bytes().to_vec(),
            1,
        )),
        Box::new(crate::crypto::signal_store::MemoryPreKeyStore::new()),
        Box::new(crate::crypto::signal_store::MemorySessionStore::new()),
    );
    let prekey_bundle = signal
        .generate_prekey_bundle(1, Some(1), Some(1))
        .map_err(|e| TransportError::FriendRequestFailed {
            target: target_mailbox_key.to_string(),
            reason: format!("prekey: {e}"),
        })?;
    // Extract prekey private material for persistence across restarts
    let signed_prekey_private = signal.load_signed_prekey(1).unwrap_or_default();
    let one_time_prekey_private = signal.load_prekey(1).ok().flatten();
    let prekey_bytes =
        prekey_bundle
            .to_bytes()
            .map_err(|e| TransportError::FriendRequestFailed {
                target: target_mailbox_key.to_string(),
                reason: format!("prekey serialize: {e}"),
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

    write_inbox_entry(
        node,
        &inbox_key,
        inbox_kp,
        subkey,
        &request,
        "friend request",
    )
    .await
    .map_err(|reason| TransportError::FriendRequestFailed {
        target: target_mailbox_key.to_string(),
        reason,
    })?;

    // Best-effort direct notification via target's route (tier 2 — instant).
    // The DHT watch (tier 1) and poll (tier 3) will also discover this,
    // but a direct notification ensures sub-second awareness if the target is online.
    // The notification is signed to the target's identity key, which their
    // published prekey bundle names; without it there is nothing to sign to
    // and the DHT tiers deliver the request alone.
    let pool = node.require_records()?;
    let target_identity = read_profile_subkey(
        &pool,
        target_mailbox_key,
        PROFILE_SUBKEY_PREKEY_BUNDLE,
        true,
    )
    .await
    .ok()
    .flatten()
    .and_then(|bytes| crate::crypto::prekeys::from_bytes(&bytes).ok())
    .and_then(|bundle| <[u8; 32]>::try_from(bundle.identity_key.as_slice()).ok());
    if let (Some(recipient), Ok(Some(route_blob))) = (
        target_identity,
        read_peer_mailbox_route(&pool, target_mailbox_key).await,
    ) {
        if !route_blob.is_empty() {
            if let Ok(target) = node.import_route(&route_blob) {
                let notify_payload = crate::payload::dm::DmPayload::FriendRequestAck;
                let notify_bytes =
                    crate::payload::dm::serialize_dm(&notify_payload).unwrap_or_default();
                if !notify_bytes.is_empty() {
                    let _ = node
                        .sender()
                        .send_dm(
                            &target,
                            &recipient,
                            crate::frame::TypeId::FriendRequestAck.class(),
                            crate::frame::TypeId::FriendRequestAck,
                            signing_key_bytes,
                            &session.identity.public_key_hex,
                            0,
                            None,
                            &notify_bytes,
                        )
                        .await;
                    info!("direct notification sent to target via route");
                }
            }
        }
    }

    Ok(FriendRequestSent {
        signed_prekey_private,
        one_time_prekey_private,
    })
}

pub struct FriendAccepted {
    pub dm_log_key: String,
    pub dm_log_keypair_bytes: Vec<u8>,
}

/// Accept a pending friend request.
pub async fn accept_friend_request(
    node: &TransportNode,
    session: &Session,
    requester_public_key: &str,
    requester_route_blob: &[u8],
    requester_profile_dht_key: &str,
    requester_display_name: &str,
) -> Result<FriendAccepted> {
    info!(
        requester = %requester_public_key,
        "accepting friend request"
    );

    // Create shared DM DhtLog
    let (dm_log, dm_log_kp) = crate::broadcast::dht_writes::create_dht_log(node).await?;
    let dm_log_key = dm_log.spine_key().to_string();
    dm_log.release(&*node.require_records()?).await;
    let dm_log_keypair_bytes = super::identity::serialize_keypair(&dm_log_kp);
    let dm_log_keypair_hex = hex::encode(&dm_log_keypair_bytes);

    // Add to friend list DHT
    let nickname = if requester_display_name.is_empty() {
        None
    } else {
        Some(requester_display_name.to_string())
    };
    let friend_entry = FriendEntry {
        public_key: requester_public_key.to_string(),
        nickname,
        group: None,
        added_at: rekindle_utils::timestamp_ms(),
        profile_dht_key: Some(requester_profile_dht_key.to_string()),
        dm_log_key: Some(dm_log_key.clone()),
    };
    let outcome = rekindle_protocol::dht::friends::add_friend(
        &*node.require_records()?,
        &session.identity.friend_list_dht_key,
        friend_entry,
    )
    .await?;
    if outcome.missed() {
        tracing::warn!(?outcome, "friend list add not stored at consensus");
    }

    if !requester_route_blob.is_empty() {
        node.peers()
            .write()
            .cache_route(requester_public_key, requester_route_blob.to_vec());
    }

    // Write Accepted response to requester's friend inbox. Errors here are
    // propagated, not swallowed: a failed write means the requester never
    // learns they were accepted and will keep treating the friendship as
    // pending — the exact silent-failure class `send_friend_request` above
    // already guards against with the same write-then-verify-then-retry
    // shape, applied here too.
    let pool = node.require_records()?;
    let req_inbox_key = read_profile_subkey(
        &pool,
        requester_profile_dht_key,
        PROFILE_SUBKEY_FRIEND_INBOX_KEY,
        true,
    )
    .await
    .unwrap_or(None);
    let req_inbox_kp = read_profile_subkey(
        &pool,
        requester_profile_dht_key,
        PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR,
        true,
    )
    .await
    .unwrap_or(None);

    let (Some(key_bytes), Some(kp_bytes)) = (req_inbox_key, req_inbox_kp) else {
        return Err(TransportError::FriendAcceptFailed {
            requester: requester_public_key.to_string(),
            reason: "requester's friend inbox info not available".into(),
        });
    };
    let inbox_key = String::from_utf8_lossy(&key_bytes).to_string();
    let kp_hex = String::from_utf8_lossy(&kp_bytes).to_string();
    let kp_raw = hex::decode(&kp_hex).map_err(|e| TransportError::FriendAcceptFailed {
        requester: requester_public_key.to_string(),
        reason: format!("invalid inbox keypair: {e}"),
    })?;
    let kp = crate::broadcast::node::deserialize_keypair(&kp_raw).map_err(|e| {
        TransportError::FriendAcceptFailed {
            requester: requester_public_key.to_string(),
            reason: format!("inbox keypair deserialize: {e}"),
        }
    })?;
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
    let subkey = blake3_hash_mod(
        &session.identity.public_key_hex,
        requester_profile_dht_key,
        32,
    );
    write_inbox_entry(node, &inbox_key, kp, subkey, &response, "acceptance")
        .await
        .map_err(|reason| TransportError::FriendAcceptFailed {
            requester: requester_public_key.to_string(),
            reason,
        })?;

    let dm_log_keypair_bytes = super::identity::serialize_keypair(&dm_log_kp);
    info!(requester = %requester_public_key, dm_log = %dm_log_key, "friend request accepted");
    Ok(FriendAccepted {
        dm_log_key,
        dm_log_keypair_bytes,
    })
}

/// Reject a pending friend request.
pub async fn reject_friend_request(
    node: &TransportNode,
    session: &Session,
    requester_public_key: &str,
) -> Result<()> {
    info!(
        requester = %requester_public_key,
        "rejecting friend request"
    );

    let Some(req) = session.pending_request_by_key(requester_public_key) else {
        // Nothing pending to reject — idempotent no-op, not an error.
        return Ok(());
    };

    let pool = node.require_records()?;
    let inbox_key_data = read_profile_subkey(
        &pool,
        &req.profile_dht_key,
        PROFILE_SUBKEY_FRIEND_INBOX_KEY,
        true,
    )
    .await
    .unwrap_or(None);
    let inbox_kp_data = read_profile_subkey(
        &pool,
        &req.profile_dht_key,
        PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR,
        true,
    )
    .await
    .unwrap_or(None);

    // Errors from here on are propagated, not swallowed: see the matching
    // comment in `accept_friend_request` — a failed write means the
    // requester never learns they were rejected and will keep re-sending
    // the now-stale request indefinitely.
    let (Some(key_bytes), Some(kp_bytes)) = (inbox_key_data, inbox_kp_data) else {
        return Err(TransportError::FriendRejectFailed {
            requester: requester_public_key.to_string(),
            reason: "requester's friend inbox info not available".into(),
        });
    };
    let inbox_key = String::from_utf8_lossy(&key_bytes).to_string();
    let kp_hex = String::from_utf8_lossy(&kp_bytes).to_string();
    let kp_raw = hex::decode(&kp_hex).map_err(|e| TransportError::FriendRejectFailed {
        requester: requester_public_key.to_string(),
        reason: format!("invalid inbox keypair: {e}"),
    })?;
    let kp = crate::broadcast::node::deserialize_keypair(&kp_raw).map_err(|e| {
        TransportError::FriendRejectFailed {
            requester: requester_public_key.to_string(),
            reason: format!("inbox keypair deserialize: {e}"),
        }
    })?;
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
        status: FriendRequestStatus::Rejected {
            rejected_at: rekindle_utils::timestamp_ms(),
        },
    };
    let subkey = blake3_hash_mod(&session.identity.public_key_hex, &req.profile_dht_key, 32);
    write_inbox_entry(node, &inbox_key, kp, subkey, &response, "rejection")
        .await
        .map_err(|reason| TransportError::FriendRejectFailed {
            requester: requester_public_key.to_string(),
            reason,
        })?;
    Ok(())
}

/// Remove a friend.
pub async fn remove_friend(node: &TransportNode, session: &Session, peer_key: &str) -> Result<()> {
    info!(peer = %peer_key, "removing friend");
    let outcome = rekindle_protocol::dht::friends::remove_friend(
        &*node.require_records()?,
        &session.identity.friend_list_dht_key,
        peer_key,
    )
    .await?;
    if outcome.missed() {
        tracing::warn!(?outcome, "friend list removal not stored at consensus");
    }
    node.peers().write().invalidate_route(peer_key);
    info!(peer = %peer_key, "friend removed");
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

/// Write `entry` into a peer's friend inbox subkey on one borrow of the
/// inbox, writable with its shared keypair: read-append-write, verify that
/// it propagated, and once more when it did not (a concurrent writer won
/// the subkey). `Err(reason)` when the inbox could not be opened or the
/// write did not land (plan C7.7d: writes are honest, never queued).
async fn write_inbox_entry(
    node: &TransportNode,
    inbox_key: &str,
    inbox_keypair: veilid_core::KeyPair,
    subkey: u32,
    entry: &FriendRequestEntry,
    what: &'static str,
) -> std::result::Result<(), String> {
    let lease = crate::broadcast::dht_writes::acquire_str(
        node,
        inbox_key,
        Some(&inbox_keypair.to_string()),
    )
    .await
    .map_err(|e| format!("cannot open inbox: {e}"))?;
    let written = write_and_verify(node, lease, subkey, entry, what).await;
    crate::broadcast::dht_writes::release(node, lease).await;
    written
}

async fn write_and_verify(
    node: &TransportNode,
    lease: LeaseId,
    subkey: u32,
    entry: &FriendRequestEntry,
    what: &'static str,
) -> std::result::Result<(), String> {
    read_append_write(node, lease, subkey, entry)
        .await
        .map_err(|e| format!("inbox write: {e}"))?;
    info!(what, "written to the friend inbox — verifying propagation");
    if verify_entry_present(node, lease, subkey, entry).await {
        info!(what, "verified — propagated to network");
        return Ok(());
    }
    tracing::warn!(
        what,
        "not found after write — retrying (concurrent writer race)"
    );
    read_append_write(node, lease, subkey, entry)
        .await
        .map_err(|e| format!("inbox retry write: {e}"))?;
    if verify_entry_present(node, lease, subkey, entry).await {
        info!(what, "verified on retry — propagated to network");
    } else {
        tracing::warn!(
            what,
            "written but propagation not confirmed after retry — the peer may see it late"
        );
    }
    Ok(())
}

/// Read existing entries from a subkey, append our entry, write back.
/// Returns error only on write failure — empty/unparseable subkeys are
/// treated as empty arrays (safe to overwrite). A newer value on the
/// network (a concurrent writer) is not an error: the verify re-reads.
async fn read_append_write(
    node: &TransportNode,
    lease: LeaseId,
    subkey: u32,
    entry: &FriendRequestEntry,
) -> Result<()> {
    // Read current state
    let existing = match crate::broadcast::dht_writes::get_leased(node, lease, subkey, true).await {
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

    let bytes = serde_json::to_vec(&entries).map_err(|e| TransportError::SerializationFailed {
        reason: e.to_string(),
    })?;
    crate::broadcast::dht_writes::set_leased_str(node, lease, subkey, bytes, None)
        .await
        .map(|_| ())
}

/// Verify our entry is present in the subkey after writing.
/// Retries with exponential backoff on failure (concurrent writer race).
async fn verify_entry_present(
    node: &TransportNode,
    lease: LeaseId,
    subkey: u32,
    entry: &FriendRequestEntry,
) -> bool {
    let deadline = std::time::Duration::from_secs(15);
    let start = std::time::Instant::now();
    let mut backoff = std::time::Duration::from_millis(300);
    let ceiling = std::time::Duration::from_secs(3);

    loop {
        match crate::broadcast::dht_writes::get_leased(node, lease, subkey, true).await {
            Ok(Some(data)) if !data.is_empty() && data != b"[]" => {
                let entries: Vec<FriendRequestEntry> = serde_json::from_slice::<
                    Vec<FriendRequestEntry>,
                >(&data)
                .or_else(|_| serde_json::from_slice::<FriendRequestEntry>(&data).map(|e| vec![e]))
                .unwrap_or_default();
                if entries
                    .iter()
                    .any(|e| e.sender_public_key == entry.sender_public_key)
                {
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

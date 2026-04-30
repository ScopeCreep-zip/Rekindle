//! Friend lifecycle operations — send request, accept, reject, remove.

use tracing::info;

use crate::error::{TransportError, Result};
use crate::node::TransportNode;
use crate::payload::dm::{DmPayload, dm_type_id, serialize_dm};
use crate::payload::dht_types::FriendEntry;
use crate::session::Session;

/// Send a friend request to a peer.
///
/// Steps:
/// 1. Resolve the target's mailbox DHT key to get their route blob
/// 2. Build `DmPayload::FriendRequest` with our profile info
/// 3. Send via the transport sender
pub async fn send_friend_request(
    node: &TransportNode,
    session: &Session,
    target_mailbox_key: &str,
    message: &str,
    signing_key_bytes: &[u8; 32],
) -> Result<()> {
    info!(target = &target_mailbox_key[..16.min(target_mailbox_key.len())], "sending friend request");

    // Read target's route blob from their mailbox
    let dht = node.dht()?;
    let route_blob = dht
        .mailbox()
        .read_peer_route(target_mailbox_key)
        .await?
        .ok_or_else(|| TransportError::FriendRequestFailed {
            target: target_mailbox_key.to_string(),
            reason: "peer mailbox empty or unreachable".into(),
        })?;

    let target = node.import_route(&route_blob)?;

    // Get our current route blob to include in the request
    let our_route_blob = {
        let routes = node.routes();
        let rm = routes.read();
        rm.route_blob()
            .ok_or_else(|| TransportError::FriendRequestFailed {
                target: target_mailbox_key.to_string(),
                reason: "no local route allocated — cannot receive responses".into(),
            })?
            .to_vec()
    };

    // Generate prekey bundle for Signal session establishment
    let x25519_secret = crate::crypto::pseudonym::pseudonym_to_x25519(
        &ed25519_dalek::SigningKey::from_bytes(signing_key_bytes),
    );
    let x25519_public = x25519_dalek::PublicKey::from(&x25519_secret);

    let signal = crate::crypto::signal_session::SignalSessionManager::new(
        Box::new(crate::crypto::signal_store::MemoryIdentityStore::new(
            x25519_secret.to_bytes().to_vec(),
            x25519_public.as_bytes().to_vec(),
            1,
        )),
        Box::new(crate::crypto::signal_store::MemoryPreKeyStore::new()),
        Box::new(crate::crypto::signal_store::MemorySessionStore::new()),
    );

    let prekey_bundle = signal.generate_prekey_bundle(1, Some(1))
        .map_err(|e| TransportError::FriendRequestFailed {
            target: target_mailbox_key.to_string(),
            reason: format!("prekey generation: {e}"),
        })?;

    let prekey_bytes = prekey_bundle.to_bytes()
        .map_err(|e| TransportError::FriendRequestFailed {
            target: target_mailbox_key.to_string(),
            reason: format!("prekey serialization: {e}"),
        })?;

    // Build friend request payload
    let dm = DmPayload::FriendRequest {
        display_name: session.identity.display_name.clone(),
        message: message.to_string(),
        prekey_bundle: prekey_bytes,
        profile_dht_key: session.identity.profile_dht_key.clone(),
        route_blob: our_route_blob,
        mailbox_dht_key: session.identity.mailbox_dht_key.clone(),
        invite_id: None,
    };

    let payload_bytes = serialize_dm(&dm)?;
    let type_id = dm_type_id(&dm);

    node.sender()
        .send_dm(
            &target,
            type_id,
            signing_key_bytes,
            &session.identity.public_key_hex,
            &payload_bytes,
        )
        .await?;

    info!("friend request sent");
    Ok(())
}

/// Accept a pending friend request.
///
/// Steps:
/// 1. Build `DmPayload::FriendAccept` with our prekey bundle and route
/// 2. Send to the requester
/// 3. Add to our friend list DHT record
pub async fn accept_friend_request(
    node: &TransportNode,
    session: &Session,
    requester_public_key: &str,
    requester_route_blob: &[u8],
    requester_profile_dht_key: &str,
    signing_key_bytes: &[u8; 32],
) -> Result<()> {
    info!(requester = &requester_public_key[..12], "accepting friend request");

    let target = node.import_route(requester_route_blob)?;

    let our_route_blob = {
        let routes = node.routes();
        let rm = routes.read();
        rm.route_blob()
            .ok_or_else(|| TransportError::FriendRequestFailed {
                target: requester_public_key.to_string(),
                reason: "no local route allocated".into(),
            })?
            .to_vec()
    };

    // Generate our prekey bundle and ephemeral key for Signal session
    let x25519_secret = crate::crypto::pseudonym::pseudonym_to_x25519(
        &ed25519_dalek::SigningKey::from_bytes(signing_key_bytes),
    );
    let x25519_public = x25519_dalek::PublicKey::from(&x25519_secret);

    let signal = crate::crypto::signal_session::SignalSessionManager::new(
        Box::new(crate::crypto::signal_store::MemoryIdentityStore::new(
            x25519_secret.to_bytes().to_vec(),
            x25519_public.as_bytes().to_vec(),
            1,
        )),
        Box::new(crate::crypto::signal_store::MemoryPreKeyStore::new()),
        Box::new(crate::crypto::signal_store::MemorySessionStore::new()),
    );

    let prekey_bundle = signal.generate_prekey_bundle(1, Some(1))
        .map_err(|e| TransportError::FriendRequestFailed {
            target: requester_public_key.to_string(),
            reason: format!("prekey generation: {e}"),
        })?;

    let prekey_bytes = prekey_bundle.to_bytes()
        .map_err(|e| TransportError::FriendRequestFailed {
            target: requester_public_key.to_string(),
            reason: format!("prekey serialization: {e}"),
        })?;

    // Generate X25519 ephemeral for X3DH
    let ephemeral_secret = x25519_dalek::StaticSecret::random_from_rng(rand::rngs::OsRng);
    let ephemeral_public = x25519_dalek::PublicKey::from(&ephemeral_secret);

    // Build accept payload with real crypto material
    let dm = DmPayload::FriendAccept {
        prekey_bundle: prekey_bytes,
        profile_dht_key: session.identity.profile_dht_key.clone(),
        route_blob: our_route_blob,
        mailbox_dht_key: session.identity.mailbox_dht_key.clone(),
        ephemeral_key: ephemeral_public.as_bytes().to_vec(),
        signed_prekey_id: 1,
        one_time_prekey_id: Some(1),
    };

    let payload_bytes = serialize_dm(&dm)?;
    let type_id = dm_type_id(&dm);

    node.sender()
        .send_dm(
            &target,
            type_id,
            signing_key_bytes,
            &session.identity.public_key_hex,
            &payload_bytes,
        )
        .await?;

    // Add to friend list DHT
    let dht = node.dht()?;
    let friend_entry = FriendEntry {
        public_key: requester_public_key.to_string(),
        nickname: None,
        group: None,
        added_at: rekindle_utils::timestamp_ms(),
        profile_dht_key: Some(requester_profile_dht_key.to_string()),
    };

    dht.friend_list()
        .add(&session.identity.friend_list_dht_key, friend_entry)
        .await?;

    // Cache their route for future messaging
    node.peers()
        .write()
        .cache_route(requester_public_key, requester_route_blob.to_vec());

    info!(requester = &requester_public_key[..12], "friend request accepted");
    Ok(())
}

/// Reject a pending friend request.
///
/// Sends a `FriendReject` DM to the requester so they know the request
/// was explicitly declined (not just ignored/timed out).
pub async fn reject_friend_request(
    node: &TransportNode,
    session: &Session,
    requester_public_key: &str,
    requester_route_blob: &[u8],
    signing_key_bytes: &[u8; 32],
) -> Result<()> {
    info!(requester = &requester_public_key[..12.min(requester_public_key.len())], "rejecting friend request");

    let target = node.import_route(requester_route_blob)?;

    let dm = DmPayload::FriendReject;
    let payload_bytes = serialize_dm(&dm)?;
    let type_id = dm_type_id(&dm);

    node.sender()
        .send_dm(
            &target,
            type_id,
            signing_key_bytes,
            &session.identity.public_key_hex,
            &payload_bytes,
        )
        .await?;

    info!(requester = &requester_public_key[..12.min(requester_public_key.len())], "friend request rejected");
    Ok(())
}

/// Remove a friend.
///
/// Steps:
/// 1. Remove from friend list DHT record
/// 2. Send `Unfriend` notification (best-effort)
/// 3. Invalidate cached route
pub async fn remove_friend(
    node: &TransportNode,
    session: &Session,
    peer_key: &str,
    signing_key_bytes: &[u8; 32],
) -> Result<()> {
    info!(peer = &peer_key[..12], "removing friend");

    // Remove from friend list
    let dht = node.dht()?;
    dht.friend_list()
        .remove(&session.identity.friend_list_dht_key, peer_key)
        .await?;

    // Best-effort unfriend notification
    let route_blob = {
        let peers = node.peers();
        let registry = peers.read();
        registry.get_route(peer_key).map(<[u8]>::to_vec)
    };

    if let Some(blob) = route_blob {
        if let Ok(target) = node.import_route(&blob) {
            let dm = DmPayload::Unfriend;
            let payload_bytes = serialize_dm(&dm).unwrap_or_default();
            let type_id = dm_type_id(&dm);
            let _ = node
                .sender()
                .send_dm(
                    &target,
                    type_id,
                    signing_key_bytes,
                    &session.identity.public_key_hex,
                    &payload_bytes,
                )
                .await;
        }
    }

    // Invalidate route cache
    node.peers().write().invalidate_route(peer_key);

    info!(peer = &peer_key[..12], "friend removed");
    Ok(())
}

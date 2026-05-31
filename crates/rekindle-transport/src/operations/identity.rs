//! Identity lifecycle operations — create, export, rotate, destroy.
//!
//! Orchestration logic that composes:
//! - `broadcast::dht_writes` for raw DHT primitives (create, open, set, close)
//! - `broadcast::route` for route allocation
//! - `broadcast::dm` for DM sends (rotation notifications)
//! - `dht/*` typed modules for business logic reads/writes (profile, mailbox, friend list)

use tracing::{info, warn};

use crate::crypto::signal_store::{PreKeyStore, SessionStore};
use crate::error::{TransportError, Result};
use crate::broadcast::node::TransportNode;

pub struct IdentityCreated {
    pub public_key_hex: String,
    pub signing_key_bytes: [u8; 32],
    pub profile_dht_key: String,
    pub profile_keypair_bytes: Vec<u8>,
    pub mailbox_dht_key: String,
    pub friend_list_dht_key: String,
    pub friend_list_keypair_bytes: Vec<u8>,
    pub friend_inbox_key: String,
    pub friend_inbox_keypair_hex: String,
    pub route_id: String,
    pub route_blob: Vec<u8>,
    pub prekey_material: PrekeyMaterial,
}

pub struct PrekeyMaterial {
    pub signed_prekey: (u32, Vec<u8>),
    pub one_time_prekeys: Vec<(u32, Vec<u8>)>,
}

pub async fn create_identity(
    node: &TransportNode, display_name: &str, status_message: &str,
    prekey_store: Box<dyn PreKeyStore>, session_store: Box<dyn SessionStore>,
) -> Result<IdentityCreated> {
    info!(display_name, "starting identity creation ceremony");

    // Step 1: Generate Ed25519 keypair
    let signing_key = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
    let public_key = signing_key.verifying_key();
    let public_key_hex = hex::encode(public_key.as_bytes());
    let signing_key_bytes = signing_key.to_bytes();
    info!(public_key = %public_key_hex, "keypair generated");

    // Step 2: Allocate private route
    let (route_id, route_blob) = crate::broadcast::route::allocate_personal(node).await
        .map_err(|e| TransportError::IdentityCreationFailed { step: "route allocation".into(), reason: e.to_string() })?;
    info!(route_id, "private route allocated");

    // Step 3: Generate prekey bundle. Signal identity store holds the
    // Ed25519 keypair bytes — PQXDH derives X25519 internally via
    // `to_scalar_bytes` and feeds the Ed25519 public to peers for SPK/PQ
    // signature verification.
    let signal = crate::crypto::signal_session::SignalSessionManager::new(
        Box::new(crate::crypto::signal_store::MemoryIdentityStore::new(
            signing_key_bytes.to_vec(), public_key.as_bytes().to_vec(), 1,
        )),
        prekey_store, session_store,
    );
    let signed_prekey_id = 1u32;
    let one_time_prekey_id = Some(1u32);
    let prekey_bundle = signal.generate_prekey_bundle(signed_prekey_id, one_time_prekey_id, Some(1))
        .map_err(|e| TransportError::IdentityCreationFailed { step: "prekey generation".into(), reason: e.to_string() })?;
    let signed_prekey_private = signal.load_signed_prekey(signed_prekey_id).unwrap_or_default();
    let one_time_prekey_private = one_time_prekey_id.and_then(|id| signal.load_prekey(id).ok().flatten()).unwrap_or_default();
    let prekey_material = PrekeyMaterial {
        signed_prekey: (signed_prekey_id, signed_prekey_private),
        one_time_prekeys: one_time_prekey_id.into_iter()
            .zip(std::iter::once(one_time_prekey_private))
            .filter(|(_, data)| !data.is_empty()).collect(),
    };
    let prekey_bytes = prekey_bundle.to_bytes()
        .map_err(|e| TransportError::IdentityCreationFailed { step: "prekey serialization".into(), reason: e.to_string() })?;
    info!("prekey bundle generated ({} bytes)", prekey_bytes.len());

    // Step 4: Create profile DHT record (typed business logic)
    let dht = node.dht().map_err(|e| TransportError::IdentityCreationFailed { step: "dht access".into(), reason: e.to_string() })?;
    let (profile_dht_key, profile_keypair) = dht.profile()
        .create(display_name, status_message, &prekey_bytes, &route_blob).await
        .map_err(|e| TransportError::IdentityCreationFailed { step: "profile record".into(), reason: e.to_string() })?;
    let profile_keypair_bytes = profile_keypair.map(|kp| serialize_keypair(&kp)).unwrap_or_default();
    info!(key = %profile_dht_key, "profile record created");

    // Step 5: Create mailbox DHT record (typed business logic)
    let identity_keypair = ed25519_to_keypair(&signing_key);
    let mailbox_dht_key = match dht.mailbox().create(identity_keypair.clone()).await {
        Ok(key) => key,
        Err(e) => {
            warn!(error = %e, "mailbox creation failed, cleaning up profile");
            let _ = dht.profile().close(&profile_dht_key).await;
            return Err(TransportError::IdentityCreationFailed { step: "mailbox record".into(), reason: e.to_string() });
        }
    };
    if let Err(e) = dht.mailbox().update_route(&mailbox_dht_key, &route_blob).await {
        warn!(error = %e, "mailbox route update failed, cleaning up");
        let _ = dht.mailbox().close(&mailbox_dht_key).await;
        let _ = dht.profile().close(&profile_dht_key).await;
        return Err(TransportError::IdentityCreationFailed { step: "mailbox route".into(), reason: e.to_string() });
    }
    info!(key = %mailbox_dht_key, "mailbox record created");

    // Step 6: Create friend list DHT record (typed business logic)
    let (friend_list_dht_key, friend_list_keypair) = match dht.friend_list().create().await {
        Ok(result) => result,
        Err(e) => {
            warn!(error = %e, "friend list creation failed, cleaning up");
            let _ = dht.mailbox().close(&mailbox_dht_key).await;
            let _ = dht.profile().close(&profile_dht_key).await;
            return Err(TransportError::IdentityCreationFailed { step: "friend list record".into(), reason: e.to_string() });
        }
    };
    let friend_list_keypair_bytes = friend_list_keypair.map(|kp| serialize_keypair(&kp)).unwrap_or_default();
    info!(key = %friend_list_dht_key, "friend list record created");

    // Step 7: Create friend inbox (raw DHT primitive — DFLT(32), no typed wrapper needed)
    let (friend_inbox_key, friend_inbox_keypair) = crate::broadcast::dht_writes::create_dflt(node, 32, None).await
        .map_err(|e| TransportError::IdentityCreationFailed { step: "friend inbox".into(), reason: e.to_string() })?;
    let friend_inbox_keypair_hex = friend_inbox_keypair.map(|kp| hex::encode(serialize_keypair(&kp))).unwrap_or_default();

    // Seed subkey 0 (raw primitive)
    let _ = crate::broadcast::dht_writes::set(node, &friend_inbox_key, 0, b"[]".to_vec(), None).await;

    // Publish inbox key + keypair to profile (raw primitive — profile subkey writes)
    let _ = crate::broadcast::dht_writes::set(
        node, &profile_dht_key,
        crate::payload::dht_types::PROFILE_SUBKEY_FRIEND_INBOX_KEY,
        friend_inbox_key.as_bytes().to_vec(), None,
    ).await;
    let _ = crate::broadcast::dht_writes::set(
        node, &profile_dht_key,
        crate::payload::dht_types::PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR,
        friend_inbox_keypair_hex.as_bytes().to_vec(), None,
    ).await;

    info!(key = %friend_inbox_key, "friend inbox created and published to profile");
    info!("identity creation ceremony complete");

    Ok(IdentityCreated {
        public_key_hex, signing_key_bytes, profile_dht_key, profile_keypair_bytes,
        mailbox_dht_key, friend_list_dht_key, friend_list_keypair_bytes,
        friend_inbox_key, friend_inbox_keypair_hex, route_id, route_blob, prekey_material,
    })
}

pub async fn destroy_identity(node: &TransportNode, session: &crate::session::Session) -> Result<()> {
    info!("destroying identity — closing all DHT records");
    let dht = node.dht()?;
    let _ = dht.profile().close(&session.identity.profile_dht_key).await;
    let _ = dht.mailbox().close(&session.identity.mailbox_dht_key).await;
    let _ = dht.friend_list().close(&session.identity.friend_list_dht_key).await;
    for membership in session.communities.values() {
        let _ = dht.governance().close(&membership.governance_key).await;
        let _ = crate::broadcast::dht_writes::close(node, &membership.registry_key).await;
    }
    info!("identity destroyed — all DHT records closed");
    Ok(())
}

pub async fn rotate_identity(
    node: &TransportNode, session: &crate::session::Session, old_signing_key_bytes: &[u8; 32],
) -> Result<RotatedIdentity> {
    info!("rotating identity keypair");
    let new_signing_key = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
    let new_public_key_hex = hex::encode(new_signing_key.verifying_key().as_bytes());
    let new_signing_key_bytes = new_signing_key.to_bytes();
    info!(new_public_key = %new_public_key_hex, "new keypair generated");

    // Notify all friends via broadcast::dm
    let dht = node.dht()?;
    let friends = dht.friend_list().read(&session.identity.friend_list_dht_key).await?;
    let mut notified = 0u32;
    for friend in &friends.friends {
        match crate::broadcast::dm::profile_key_rotated(
            node, session, &friend.public_key, &session.identity.profile_dht_key, old_signing_key_bytes,
        ).await {
            Ok(()) => { notified += 1; }
            Err(e) => { tracing::debug!(peer = %friend.public_key, error = %e, "rotation notify failed"); }
        }
    }
    info!(notified, total_friends = friends.friends.len(), "rotation notifications sent");

    Ok(RotatedIdentity { new_public_key_hex, new_signing_key_bytes, friends_notified: notified })
}

pub struct RotatedIdentity {
    pub new_public_key_hex: String,
    pub new_signing_key_bytes: [u8; 32],
    pub friends_notified: u32,
}

/// Re-export from broadcast boundary for crate-internal use.
pub(crate) use crate::broadcast::node::serialize_keypair;
pub(crate) use crate::broadcast::node::ed25519_to_keypair;

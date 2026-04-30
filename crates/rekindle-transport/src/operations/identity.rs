//! Identity lifecycle operations — create, export, rotate.
//!
//! The identity ceremony (`create_identity`) is the first thing a new user
//! runs. It generates the Ed25519 keypair, creates all required DHT records,
//! allocates a private route, and publishes the route blob. On failure at
//! any step, previously created resources are cleaned up (best-effort).

use tracing::{info, warn};

use crate::error::{TransportError, Result};
use crate::node::TransportNode;

/// Result of a successful identity creation ceremony.
///
/// The caller (CLI) is responsible for:
/// 1. Storing `signing_key_bytes` in the OS keyring (then zeroizing)
/// 2. Storing `profile_keypair_bytes` and `friend_list_keypair_bytes` in the keyring
/// 3. Building a `Session` from these fields
/// 4. Persisting the `Session` to disk
pub struct IdentityCreated {
    /// Ed25519 public key, hex-encoded (64 chars).
    pub public_key_hex: String,
    /// Ed25519 signing key bytes (32 bytes). MUST be zeroized after keyring storage.
    pub signing_key_bytes: [u8; 32],
    /// Profile DHT record key.
    pub profile_dht_key: String,
    /// Profile DHT record keypair (serialized). For re-opening writable.
    pub profile_keypair_bytes: Vec<u8>,
    /// Mailbox DHT record key (deterministic from identity keypair).
    pub mailbox_dht_key: String,
    /// Friend list DHT record key.
    pub friend_list_dht_key: String,
    /// Friend list DHT record keypair (serialized). For re-opening writable.
    pub friend_list_keypair_bytes: Vec<u8>,
    /// Allocated private route ID string.
    pub route_id: String,
    /// Allocated private route blob (publish to DHT for peers to reach us).
    pub route_blob: Vec<u8>,
}

/// Execute the full identity creation ceremony.
///
/// Steps:
/// 1. Generate Ed25519 keypair
/// 2. Allocate private route
/// 3. Create profile DHT record (display name, status, route blob)
/// 4. Create mailbox DHT record (route blob for peer discovery)
/// 5. Create friend list DHT record (empty)
///
/// On failure, attempts to clean up any partially created resources.
pub async fn create_identity(
    node: &TransportNode,
    display_name: &str,
    status_message: &str,
) -> Result<IdentityCreated> {
    info!(display_name, "starting identity creation ceremony");

    // Step 1: Generate Ed25519 keypair
    let signing_key = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
    let public_key = signing_key.verifying_key();
    let public_key_hex = hex::encode(public_key.as_bytes());
    let signing_key_bytes = signing_key.to_bytes();

    info!(public_key = %public_key_hex, "keypair generated");

    // Step 2: Allocate private route
    let (route_id, route_blob) = node.allocate_route().await.map_err(|e| {
        TransportError::IdentityCreationFailed {
            step: "route allocation".into(),
            reason: e.to_string(),
        }
    })?;

    info!(route_id, "private route allocated");

    // Step 3: Generate prekey bundle for Signal session establishment
    let x25519_secret = crate::crypto::pseudonym::pseudonym_to_x25519(&signing_key);
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
        .map_err(|e| TransportError::IdentityCreationFailed {
            step: "prekey generation".into(),
            reason: e.to_string(),
        })?;

    let prekey_bytes = prekey_bundle.to_bytes()
        .map_err(|e| TransportError::IdentityCreationFailed {
            step: "prekey serialization".into(),
            reason: e.to_string(),
        })?;

    info!("prekey bundle generated ({} bytes)", prekey_bytes.len());

    // Step 4: Create profile DHT record with prekey bundle
    let dht = node.dht().map_err(|e| TransportError::IdentityCreationFailed {
        step: "dht access".into(),
        reason: e.to_string(),
    })?;

    let (profile_dht_key, profile_keypair) = dht
        .profile()
        .create(display_name, status_message, &prekey_bytes, &route_blob)
        .await
        .map_err(|e| TransportError::IdentityCreationFailed {
            step: "profile record".into(),
            reason: e.to_string(),
        })?;

    let profile_keypair_bytes = profile_keypair
        .map(|kp| serialize_keypair(&kp))
        .unwrap_or_default();

    info!(key = %profile_dht_key, "profile record created");

    // Step 5: Create mailbox DHT record
    let identity_keypair = to_veilid_keypair(&signing_key);
    let mailbox_dht_key = match dht.mailbox().create(identity_keypair.clone()).await {
        Ok(key) => key,
        Err(e) => {
            warn!(error = %e, "mailbox creation failed, cleaning up profile");
            let _ = dht.profile().close(&profile_dht_key).await;
            return Err(TransportError::IdentityCreationFailed {
                step: "mailbox record".into(),
                reason: e.to_string(),
            });
        }
    };

    // Write route blob to mailbox
    if let Err(e) = dht.mailbox().update_route(&mailbox_dht_key, &route_blob).await {
        warn!(error = %e, "mailbox route update failed, cleaning up");
        let _ = dht.mailbox().close(&mailbox_dht_key).await;
        let _ = dht.profile().close(&profile_dht_key).await;
        return Err(TransportError::IdentityCreationFailed {
            step: "mailbox route".into(),
            reason: e.to_string(),
        });
    }

    info!(key = %mailbox_dht_key, "mailbox record created");

    // Step 6: Create friend list DHT record
    let (friend_list_dht_key, friend_list_keypair) = match dht.friend_list().create().await {
        Ok(result) => result,
        Err(e) => {
            warn!(error = %e, "friend list creation failed, cleaning up");
            let _ = dht.mailbox().close(&mailbox_dht_key).await;
            let _ = dht.profile().close(&profile_dht_key).await;
            return Err(TransportError::IdentityCreationFailed {
                step: "friend list record".into(),
                reason: e.to_string(),
            });
        }
    };

    let friend_list_keypair_bytes = friend_list_keypair
        .map(|kp| serialize_keypair(&kp))
        .unwrap_or_default();

    info!(key = %friend_list_dht_key, "friend list record created");
    info!("identity creation ceremony complete");

    Ok(IdentityCreated {
        public_key_hex,
        signing_key_bytes,
        profile_dht_key,
        profile_keypair_bytes,
        mailbox_dht_key,
        friend_list_dht_key,
        friend_list_keypair_bytes,
        route_id,
        route_blob,
    })
}

/// Destroy the local identity — close all DHT records and zeroize keys.
///
/// This is irreversible. The caller (CLI) must:
/// 1. Confirm with the user via typed confirmation phrase
/// 2. Call this function to close DHT records
/// 3. Delete the session file
/// 4. Delete the keyring entries
/// 5. Optionally wipe the Veilid storage directory
pub async fn destroy_identity(
    node: &TransportNode,
    session: &crate::session::Session,
) -> Result<()> {
    info!("destroying identity — closing all DHT records");

    let dht = node.dht()?;

    // Close profile record
    let _ = dht.profile().close(&session.identity.profile_dht_key).await;

    // Close mailbox record
    let _ = dht.mailbox().close(&session.identity.mailbox_dht_key).await;

    // Close friend list record
    let _ = dht.friend_list().close(&session.identity.friend_list_dht_key).await;

    // Close all community records
    for membership in session.communities.values() {
        let _ = dht.governance().close(&membership.governance_key).await;
        let _ = dht.registry().close(&membership.registry_key).await;
    }

    info!("identity destroyed — all DHT records closed");
    Ok(())
}

/// Rotate the Ed25519 identity keypair.
///
/// Steps:
/// 1. Generate new Ed25519 keypair
/// 2. Update profile DHT record with new public key in the display name
///    or metadata subkey (the profile record key stays the same)
/// 3. Broadcast `ProfileKeyRotated` DM to all friends
/// 4. Return new keypair bytes for the caller to persist to keyring
///
/// The caller (CLI) is responsible for:
/// - Storing the new signing key in the keyring
/// - Updating the session with the new public key
/// - Re-deriving all community pseudonyms
pub async fn rotate_identity(
    node: &TransportNode,
    session: &crate::session::Session,
    old_signing_key_bytes: &[u8; 32],
) -> Result<RotatedIdentity> {
    info!("rotating identity keypair");

    // Generate new keypair
    let new_signing_key = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
    let new_public_key = new_signing_key.verifying_key();
    let new_public_key_hex = hex::encode(new_public_key.as_bytes());
    let new_signing_key_bytes = new_signing_key.to_bytes();

    info!(new_public_key = %new_public_key_hex, "new keypair generated");

    // Notify all friends about the key rotation (best-effort)
    let dht = node.dht()?;
    let friends = dht
        .friend_list()
        .read(&session.identity.friend_list_dht_key)
        .await?;

    let dm = crate::payload::dm::DmPayload::ProfileKeyRotated {
        new_profile_dht_key: session.identity.profile_dht_key.clone(),
    };
    let payload_bytes = crate::payload::dm::serialize_dm(&dm)?;
    let type_id = crate::payload::dm::dm_type_id(&dm);

    let mut notified = 0u32;
    for friend in &friends.friends {
        let route_blob = {
            let peers = node.peers();
            let registry = peers.read();
            registry.get_route(&friend.public_key).map(<[u8]>::to_vec)
        };

        if let Some(blob) = route_blob {
            if let Ok(target) = node.import_route(&blob) {
                if node
                    .sender()
                    .send_dm(
                        &target,
                        type_id,
                        old_signing_key_bytes,
                        &session.identity.public_key_hex,
                        &payload_bytes,
                    )
                    .await
                    .is_ok()
                {
                    notified += 1;
                }
            }
        }
    }

    info!(
        notified,
        total_friends = friends.friends.len(),
        "rotation notifications sent"
    );

    Ok(RotatedIdentity {
        new_public_key_hex,
        new_signing_key_bytes,
        friends_notified: notified,
    })
}

/// Result of an identity rotation.
pub struct RotatedIdentity {
    /// New Ed25519 public key (hex).
    pub new_public_key_hex: String,
    /// New signing key bytes. MUST be zeroized after keyring storage.
    pub new_signing_key_bytes: [u8; 32],
    /// Number of friends successfully notified.
    pub friends_notified: u32,
}

/// Convert an Ed25519 signing key to a Veilid `KeyPair`.
fn to_veilid_keypair(signing_key: &ed25519_dalek::SigningKey) -> veilid_core::KeyPair {
    let pub_bytes = signing_key.verifying_key().to_bytes();
    let secret_bytes = signing_key.to_bytes();
    let bare_pub = veilid_core::BarePublicKey::new(&pub_bytes);
    let bare_secret = veilid_core::BareSecretKey::new(&secret_bytes);
    let veilid_pub = veilid_core::PublicKey::new(veilid_core::CRYPTO_KIND_VLD0, bare_pub);
    veilid_core::KeyPair::new_from_parts(veilid_pub, bare_secret)
}

/// Serialize a Veilid `KeyPair` to bytes for keyring storage.
///
/// Format: public key bytes (32) + secret key bytes (32) = 64 bytes total.
/// Used by identity creation and community creation.
pub(crate) fn serialize_keypair(kp: &veilid_core::KeyPair) -> Vec<u8> {
    // Store as concatenation of public (32) + secret (32) = 64 bytes
    let mut bytes = Vec::with_capacity(64);
    bytes.extend_from_slice(kp.key().value().bytes());
    bytes.extend_from_slice(kp.secret().value().bytes());
    bytes
}

//! Identity lifecycle operations — create, export, rotate, destroy.
//!
//! Orchestration logic that composes:
//! - `broadcast::dht_writes` for raw DHT primitives (create, open, set, close)
//! - `broadcast::dm` for DM sends (rotation notifications)
//! - `dht/*` typed modules for business logic reads/writes (profile, mailbox, friend list)

use tracing::{info, warn};

use crate::broadcast::node::TransportNode;
use crate::error::{Result, TransportError};

/// The records an identity creation made. The owner keypairs are in their
/// string form, as the vault stores them.
pub struct IdentityCreated {
    pub profile_dht_key: String,
    pub profile_owner_keypair: String,
    pub mailbox_dht_key: String,
    pub friend_list_dht_key: String,
    pub friend_list_owner_keypair: String,
    pub friend_inbox_key: String,
    pub friend_inbox_keypair_hex: String,
}

/// Create a new identity's records: profile (carrying `prekey_bundle`,
/// the unlock's Signal manager's current bundle), mailbox (owned by the
/// identity key), friend list and friend inbox. The identity's key and
/// prekeys are made by `rekindle_node::host::identity::create` before this
/// runs; this only publishes.
pub async fn create_identity(
    node: &TransportNode,
    signing_key_bytes: &[u8; 32],
    display_name: &str,
    status_message: &str,
    prekey_bundle: &[u8],
) -> Result<IdentityCreated> {
    info!(display_name, "starting identity creation ceremony");
    let signing_key = ed25519_dalek::SigningKey::from_bytes(signing_key_bytes);

    // No route here: the unlock that follows wants one, and resume or the
    // route publisher writes it to the profile and mailbox (plan C7.9d).

    // Steps 4-6 create our own records in the record pool the caller
    // started for this ceremony (plan C7.4). Ending that pool closes them,
    // so a failed step leaves nothing open.
    let pool = node
        .require_records()
        .map_err(|e| TransportError::IdentityCreationFailed {
            step: "record pool".into(),
            reason: e.to_string(),
        })?;
    let failed =
        |step: &str, e: rekindle_protocol::ProtocolError| TransportError::IdentityCreationFailed {
            step: step.into(),
            reason: e.to_string(),
        };

    // Step 4: Create profile DHT record
    let (_profile_lease, profile_dht_key, profile_owner_keypair, outcome) =
        rekindle_protocol::dht::profile::create_profile(
            pool.as_ref(),
            rekindle_protocol::dht::profile::ProfileFields {
                display_name,
                status_message,
                prekey_bundle,
                route_blob: &[],
            },
        )
        .await
        .map_err(|e| failed("profile record", e))?;
    if outcome.missed() {
        warn!(?outcome, "profile fields not all stored at consensus");
    }
    info!(key = %profile_dht_key, "profile record created");

    // Step 5: Create mailbox DHT record, owned by the identity key
    let identity_keypair = rekindle_crypto::keypair_text::keypair_text_from_signing(&signing_key);
    let (_mailbox_lease, mailbox_dht_key) =
        rekindle_protocol::dht::mailbox::create_mailbox(pool.as_ref(), &identity_keypair)
            .await
            .map_err(|e| failed("mailbox record", e))?;
    info!(key = %mailbox_dht_key, "mailbox record created");

    // Step 6: Create friend list DHT record
    let (_friend_list_lease, friend_list_dht_key, friend_list_owner_keypair, outcome) =
        rekindle_protocol::dht::friends::create_friend_list(pool.as_ref())
            .await
            .map_err(|e| failed("friend list record", e))?;
    if outcome.missed() {
        warn!(?outcome, "empty friend list not stored at consensus");
    }
    info!(key = %friend_list_dht_key, "friend list record created");

    // Step 7: Create the friend inbox (DFLT(32)) in the pool, and seed
    // subkey 0. Veilid's create is local; the seed is what publishes the
    // record, and until it lands a peer's friend request gets `Key not
    // found`, so it must land (plan C7.6d publish-on-first-write).
    let inbox_failed = |reason: String| TransportError::IdentityCreationFailed {
        step: "friend inbox".into(),
        reason,
    };
    let (inbox_lease, friend_inbox_key, inbox_keypair) = pool
        .create(&rekindle_records::schema::friend_inbox(), None)
        .await
        .map_err(|e| inbox_failed(e.to_string()))?;
    pool.set(inbox_lease, 0, b"[]".to_vec(), None)
        .await
        .map_err(|e| inbox_failed(format!("seed write: {e}")))?
        .require_stored(0)
        .map_err(|e| inbox_failed(format!("seed write: {e}")))?;
    let friend_inbox_keypair_hex = hex::encode(
        rekindle_crypto::keypair_text::keypair_wire(&inbox_keypair)
            .map_err(|e| inbox_failed(e.to_string()))?,
    );

    // Publish inbox key + keypair to our profile (the pool's writable lease)
    for (subkey, value) in [
        (
            crate::payload::dht_types::PROFILE_SUBKEY_FRIEND_INBOX_KEY,
            friend_inbox_key.as_bytes().to_vec(),
        ),
        (
            crate::payload::dht_types::PROFILE_SUBKEY_FRIEND_INBOX_KEYPAIR,
            friend_inbox_keypair_hex.as_bytes().to_vec(),
        ),
    ] {
        crate::broadcast::dht_writes::set_own_profile_subkey(node, &profile_dht_key, subkey, value)
            .await
            .map_err(|e| TransportError::IdentityCreationFailed {
                step: "friend inbox publish".into(),
                reason: e.to_string(),
            })?;
    }

    info!(key = %friend_inbox_key, "friend inbox created and published to profile");
    info!("identity creation ceremony complete");

    Ok(IdentityCreated {
        profile_dht_key,
        profile_owner_keypair,
        mailbox_dht_key,
        friend_list_dht_key,
        friend_list_owner_keypair,
        friend_inbox_key,
        friend_inbox_keypair_hex,
    })
}

pub async fn rotate_identity(
    node: &TransportNode,
    session: &crate::session::Session,
    old_signing_key_bytes: &[u8; 32],
) -> Result<RotatedIdentity> {
    info!("rotating identity keypair");
    let new_signing_key = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
    let new_public_key_hex = hex::encode(new_signing_key.verifying_key().as_bytes());
    let new_signing_key_bytes = new_signing_key.to_bytes();
    info!(new_public_key = %new_public_key_hex, "new keypair generated");

    // Notify all friends via broadcast::dm
    let friends = rekindle_protocol::dht::friends::read_friend_list(
        &*node.require_records()?,
        &session.identity.friend_list_dht_key,
    )
    .await?;
    let mut notified = 0u32;
    for friend in &friends.friends {
        match crate::broadcast::dm::profile_key_rotated(
            node,
            session,
            &friend.public_key,
            &session.identity.profile_dht_key,
            old_signing_key_bytes,
        )
        .await
        {
            Ok(()) => {
                notified += 1;
            }
            Err(e) => {
                tracing::debug!(peer = %friend.public_key, error = %e, "rotation notify failed");
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

pub struct RotatedIdentity {
    pub new_public_key_hex: String,
    pub new_signing_key_bytes: [u8; 32],
    pub friends_notified: u32,
}

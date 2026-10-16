//! Identity lifecycle operations — create, export, rotate, destroy.
//!
//! Orchestration logic that composes:
//! - `broadcast::dht_writes` for raw DHT primitives (create, open, set, close)
//! - `broadcast::dm` for DM sends (rotation notifications)
//! - `dht/*` typed modules for business logic reads/writes (profile, mailbox, friend list)

use tracing::{info, warn};

use crate::broadcast::node::TransportNode;
use crate::crypto::signal_store::{PreKeyStore, SessionStore};
use crate::error::{Result, TransportError};

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
    pub prekey_material: PrekeyMaterial,
}

/// Every private half behind a published prekey bundle, for the caller to
/// store before the bundle is published: a bundle whose secrets were not
/// kept cannot answer any handshake made against it (plan C7.25).
pub struct PrekeyMaterial {
    pub signed_prekey: (u32, Vec<u8>),
    pub one_time_prekeys: Vec<(u32, Vec<u8>)>,
    /// The ML-KEM-768 last-resort secret, under `PQ_LR_ID`.
    pub pq_last_resort: (u32, Vec<u8>),
    pub pq_one_time: Vec<(u32, Vec<u8>)>,
}

/// A prekey bundle and its private halves, generated over `prekey_store`
/// for the identity whose Ed25519 key is `signing_key`.
///
/// The signed prekey and the PQ last-resort key keep their fixed ids
/// (`SPK_ID`, `PQ_LR_ID`): the bundle carries no signed-prekey id, and
/// initiators on both tracks send `SPK_ID`. One-time ids are random and
/// non-zero, so a reader's 0-means-absent (`PreKeyBundle::from_wire`)
/// holds and a new bundle never reuses an id an older one handed out.
fn generate_bundle_with_material(
    signing_key: &ed25519_dalek::SigningKey,
    prekey_store: Box<dyn PreKeyStore>,
    session_store: Box<dyn SessionStore>,
) -> Result<(crate::crypto::prekeys::PreKeyBundle, PrekeyMaterial)> {
    use crate::crypto::signal_session::PQ_LR_ID;
    use crate::crypto::signal_store::PqKeyKind;
    let signal = crate::crypto::signal_session::SignalSessionManager::new(
        Box::new(crate::crypto::signal_store::MemoryIdentityStore::new(
            signing_key.to_bytes().to_vec(),
            signing_key.verifying_key().as_bytes().to_vec(),
            1,
        )),
        prekey_store,
        session_store,
    );
    let spk_id = rekindle_crypto::signal::SPK_ID;
    let otpk_id = random_one_time_id();
    let pq_ot_id = random_one_time_id();
    let bundle = signal
        .generate_prekey_bundle(spk_id, Some(otpk_id), Some(pq_ot_id))
        .map_err(|e| TransportError::IdentityCreationFailed {
            step: "prekey generation".into(),
            reason: e.to_string(),
        })?;
    let missing = |what: &str| TransportError::IdentityCreationFailed {
        step: "prekey material".into(),
        reason: format!("{what} missing right after generation"),
    };
    let material = PrekeyMaterial {
        signed_prekey: (spk_id, signal.load_signed_prekey(spk_id)?),
        one_time_prekeys: vec![(
            otpk_id,
            signal
                .load_prekey(otpk_id)?
                .ok_or_else(|| missing("one-time prekey"))?,
        )],
        pq_last_resort: (
            PQ_LR_ID,
            signal
                .load_pq_secret(PQ_LR_ID, PqKeyKind::LastResort)?
                .ok_or_else(|| missing("ML-KEM last-resort secret"))?,
        ),
        pq_one_time: vec![(
            pq_ot_id,
            signal
                .load_pq_secret(pq_ot_id, PqKeyKind::OneTime)?
                .ok_or_else(|| missing("ML-KEM one-time secret"))?,
        )],
    };
    Ok((bundle, material))
}

/// A random one-time prekey id; never 0, which the wire form reads as
/// "absent".
fn random_one_time_id() -> u32 {
    loop {
        let id = rand::random::<u32>();
        if id != 0 {
            return id;
        }
    }
}

/// A fresh bundle and its private halves for a prekey refill. The caller
/// stores every half, then publishes ([`crate::operations::mek::publish_prekey_bundle`]);
/// a storage failure must stop it before it publishes (plan C7.25).
pub fn generate_replenish_bundle(
    signing_key_bytes: &[u8; 32],
) -> Result<(crate::crypto::prekeys::PreKeyBundle, PrekeyMaterial)> {
    generate_bundle_with_material(
        &ed25519_dalek::SigningKey::from_bytes(signing_key_bytes),
        Box::new(crate::crypto::signal_store::MemoryPreKeyStore::new()),
        Box::new(crate::crypto::signal_store::MemorySessionStore::new()),
    )
}

pub async fn create_identity(
    node: &TransportNode,
    display_name: &str,
    status_message: &str,
    prekey_store: Box<dyn PreKeyStore>,
    session_store: Box<dyn SessionStore>,
) -> Result<IdentityCreated> {
    info!(display_name, "starting identity creation ceremony");

    // Step 1: Generate Ed25519 keypair
    let signing_key = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
    let public_key = signing_key.verifying_key();
    let public_key_hex = hex::encode(public_key.as_bytes());
    let signing_key_bytes = signing_key.to_bytes();
    info!(public_key = %public_key_hex, "keypair generated");

    // No route here: the unlock that follows wants one, and resume or the
    // route publisher writes it to the profile and mailbox (plan C7.9d).

    // Step 3: Generate prekey bundle. Signal identity store holds the
    // Ed25519 keypair bytes — PQXDH derives X25519 internally via
    // `to_scalar_bytes` and feeds the Ed25519 public to peers for SPK/PQ
    // signature verification. Every private half comes back for the
    // caller to keep (plan C7.25).
    let (prekey_bundle, prekey_material) =
        generate_bundle_with_material(&signing_key, prekey_store, session_store)?;
    let prekey_bytes = prekey_bundle.to_wire();
    info!("prekey bundle generated ({} bytes)", prekey_bytes.len());

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
    let (_profile_lease, profile_dht_key, profile_keypair, outcome) =
        rekindle_protocol::dht::profile::create_profile(
            &pool,
            rekindle_protocol::dht::profile::ProfileFields {
                display_name,
                status_message,
                prekey_bundle: &prekey_bytes,
                route_blob: &[],
            },
        )
        .await
        .map_err(|e| failed("profile record", e))?;
    if outcome.missed() {
        warn!(?outcome, "profile fields not all stored at consensus");
    }
    let profile_keypair_bytes = serialize_keypair(&profile_keypair);
    info!(key = %profile_dht_key, "profile record created");

    // Step 5: Create mailbox DHT record, owned by the identity key
    let identity_keypair = ed25519_to_keypair(&signing_key);
    let (_mailbox_lease, mailbox_dht_key) =
        rekindle_protocol::dht::mailbox::create_mailbox(&pool, identity_keypair)
            .await
            .map_err(|e| failed("mailbox record", e))?;
    info!(key = %mailbox_dht_key, "mailbox record created");

    // Step 6: Create friend list DHT record
    let (friend_list_dht_key, friend_list_keypair, outcome) =
        rekindle_protocol::dht::friends::create_friend_list(&pool)
            .await
            .map_err(|e| failed("friend list record", e))?;
    if outcome.missed() {
        warn!(?outcome, "empty friend list not stored at consensus");
    }
    let friend_list_keypair_bytes = serialize_keypair(&friend_list_keypair);
    info!(key = %friend_list_dht_key, "friend list record created");

    // Step 7: Create the friend inbox (DFLT(32)) in the pool, and seed
    // subkey 0. Veilid's create is local; the seed is what publishes the
    // record, and until it lands a peer's friend request gets `Key not
    // found`, so it must land (plan C7.6d publish-on-first-write).
    let inbox_failed = |reason: String| TransportError::IdentityCreationFailed {
        step: "friend inbox".into(),
        reason,
    };
    let inbox_schema = veilid_core::DHTSchema::dflt(32).map_err(|e| inbox_failed(e.to_string()))?;
    let (inbox_lease, inbox_key, inbox_keypair) = pool
        .create(inbox_schema, None)
        .await
        .map_err(|e| inbox_failed(e.to_string()))?;
    pool.set(inbox_lease, 0, b"[]".to_vec(), None)
        .await
        .and_then(|outcome| outcome.require_stored(0))
        .map_err(|e| inbox_failed(format!("seed write: {e}")))?;
    let friend_inbox_key = inbox_key.to_string();
    let friend_inbox_keypair_hex = hex::encode(serialize_keypair(&inbox_keypair));

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
        public_key_hex,
        signing_key_bytes,
        profile_dht_key,
        profile_keypair_bytes,
        mailbox_dht_key,
        friend_list_dht_key,
        friend_list_keypair_bytes,
        friend_inbox_key,
        friend_inbox_keypair_hex,
        prekey_material,
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

pub(crate) use crate::broadcast::node::ed25519_to_keypair;
/// Re-export from broadcast boundary for crate-internal use.
pub(crate) use crate::broadcast::node::serialize_keypair;

#[cfg(test)]
mod replenish_tests {
    use super::generate_replenish_bundle;

    /// The refill hands back every private half its bundle names, under
    /// the bundle's own non-zero one-time ids (plan C7.25).
    #[test]
    fn the_refill_returns_every_private_half_its_bundle_names() {
        let (bundle, material) = generate_replenish_bundle(&[5; 32]).unwrap();
        let otpk_id = bundle.one_time_prekey_id.unwrap();
        let pq_ot_id = bundle.pqpk_ot_id.unwrap();
        assert_ne!(otpk_id, 0);
        assert_ne!(pq_ot_id, 0);
        assert_eq!(material.one_time_prekeys.len(), 1);
        assert_eq!(material.one_time_prekeys[0].0, otpk_id);
        assert_eq!(material.pq_one_time.len(), 1);
        assert_eq!(material.pq_one_time[0].0, pq_ot_id);
        assert_eq!(material.signed_prekey.0, rekindle_crypto::signal::SPK_ID);
        assert_eq!(material.signed_prekey.1.len(), 32);
        assert!(!material.pq_last_resort.1.is_empty());
        assert!(!material.pq_one_time[0].1.is_empty());
    }
}

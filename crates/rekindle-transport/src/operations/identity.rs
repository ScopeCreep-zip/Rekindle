//! Identity lifecycle operations — rotate, destroy. An identity's records
//! are created by the unlock's own-records stage (`rekindle_node::host`).
//!
//! Orchestration logic that composes:
//! - `operations::friend::identity_rotation` for the notice to our friends

use tracing::info;

use crate::error::Result;
use crate::operations::friend::{self, FriendHost};

/// Mint a new identity keypair, publish its full prekey set into our same
/// record (plan E2.4: the record's owner key does not change, so friends
/// open the new identity's sessions from it), and tell our friends, over
/// the old sessions (the old key is still the unlocked one), the new
/// identity signed by both (`IdentityRotated`, plan E2.3). The Host stores
/// the new identity afterwards (`rekindle_node::host::identity::rotate`),
/// with the set's rows.
///
/// # Errors
/// No identity is unlocked.
pub async fn rotate_identity(host: &dyn FriendHost) -> Result<RotatedIdentity> {
    info!("rotating identity keypair");
    let mut secret = zeroize::Zeroizing::new([0u8; 32]);
    host.rng().fill(secret.as_mut());
    let new_signing_key = ed25519_dalek::SigningKey::from_bytes(&secret);
    let new_public_key_hex = hex::encode(new_signing_key.verifying_key().as_bytes());
    info!(new_public_key = %new_public_key_hex, "new keypair generated");

    let new_identity = rekindle_crypto::Identity::from_secret_bytes(&secret);
    // The new identity's prekeys go into the record first, so a friend's
    // first send after the notice finds them. Unpublished (the record is
    // not open), the next unlock's maintenance mints them instead.
    let set = rekindle_crypto::signal::prekey_record::mint_set(
        &new_identity,
        host.rng(),
        host.clock().now_ms_i64(),
    );
    let prekeys =
        match crate::operations::prekeys::publish(host, rekindle_db::prekeys::set_values(&set))
            .await
        {
            Ok(()) => Some(set),
            Err(e) => {
                tracing::warn!(error = %e, "the new identity's prekeys were not published");
                None
            }
        };

    // Every friend learns the new identity, signed by both, over the old
    // session, before the identity moves (plan E2.3).
    let notified = friend::identity_rotation::notify_identity_rotated(host, &new_identity).await;
    info!(notified, "rotation notifications sent");

    Ok(RotatedIdentity {
        new_public_key_hex,
        new_signing_key_bytes: new_signing_key.to_bytes(),
        friends_notified: u32::try_from(notified).unwrap_or(u32::MAX),
        prekeys,
    })
}

pub struct RotatedIdentity {
    pub new_public_key_hex: String,
    pub new_signing_key_bytes: [u8; 32],
    pub friends_notified: u32,
    /// The new identity's prekey set, when it was published: its rows are
    /// stored with the re-key.
    pub prekeys: Option<rekindle_crypto::signal::prekey_record::MintedSet>,
}

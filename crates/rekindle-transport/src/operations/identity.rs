//! Identity lifecycle operations — rotate, destroy. An identity's records
//! are created by the unlock's own-records stage (`rekindle_node::host`).
//!
//! Orchestration logic that composes:
//! - `operations::friend::identity_rotation` for the notice to our friends

use tracing::info;

use crate::error::Result;
use crate::operations::friend::{self, FriendHost};

/// Mint a new identity keypair and tell our friends, over the old
/// sessions (the old key is still the unlocked one), the new identity
/// signed by both (`IdentityRotated`, plan E2.3). The Host stores the new
/// identity afterwards (`rekindle_node::host::identity::rotate`).
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

    // Every friend learns the new identity, signed by both, over the old
    // session, before the identity moves (plan E2.3).
    let new_identity = rekindle_crypto::Identity::from_secret_bytes(&secret);
    let notified = friend::identity_rotation::notify_identity_rotated(host, &new_identity).await;
    info!(notified, "rotation notifications sent");

    Ok(RotatedIdentity {
        new_public_key_hex,
        new_signing_key_bytes: new_signing_key.to_bytes(),
        friends_notified: u32::try_from(notified).unwrap_or(u32::MAX),
    })
}

pub struct RotatedIdentity {
    pub new_public_key_hex: String,
    pub new_signing_key_bytes: [u8; 32],
    pub friends_notified: u32,
}

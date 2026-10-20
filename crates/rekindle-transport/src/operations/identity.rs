//! Identity lifecycle operations — rotate, destroy. An identity's records
//! are created by the unlock's own-records stage (`rekindle_node::host`).
//!
//! Orchestration logic that composes:
//! - `operations::friend::identity_rotation` for the notice to our friends

use tracing::info;

use crate::operations::friend::{self, FriendError, FriendHost};

/// The new identity's prekey set, minted from the journalled seed: the
/// same journal gives the same set, so the set the rotation published is
/// the one the re-key stores, however often the rotation is resumed
/// (audit 2026-10-20 R-2).
#[must_use]
pub fn mint_rotation_set(
    new_identity: &rekindle_crypto::Identity,
    mint_seed: &[u8; 32],
    issued_at_ms: i64,
) -> rekindle_crypto::signal::prekey_record::MintedSet {
    let rng = rekindle_secrets::forked_random::ForkedRandom::from_seed(mint_seed);
    rekindle_crypto::signal::prekey_record::mint_set(new_identity, &rng, issued_at_ms)
}

/// Publish the journalled new identity's full prekey set into our same
/// record (plan E2.4: the record's owner key does not change, so friends
/// open the new identity's sessions from it), then tell our friends, over
/// the old sessions (the old key is still the unlocked one), the new
/// identity signed by both (`IdentityRotated`, plan E2.3). The caller
/// journalled `secret` and `mint_seed` first; the Host stores the new
/// identity afterwards (`rekindle_node::host::identity::rotate`).
///
/// # Errors
/// No identity is unlocked, or the set was not published: no notice went
/// out (P13: a friend's first send after the notice must find the set).
pub async fn rotate_identity(
    host: &dyn FriendHost,
    secret: &[u8; 32],
    mint_seed: &[u8; 32],
    issued_at_ms: i64,
) -> Result<RotatedIdentity, FriendError> {
    info!("rotating identity keypair");
    let new_identity = rekindle_crypto::Identity::from_secret_bytes(secret);
    let new_public_key_hex = new_identity.public_key_hex();
    info!(new_public_key = %new_public_key_hex, "new keypair generated");

    let set = mint_rotation_set(&new_identity, mint_seed, issued_at_ms);
    crate::operations::prekeys::publish(host, rekindle_db::prekeys::set_values(&set)).await?;

    // Every friend learns the new identity, signed by both, over the old
    // session, before the identity moves (plan E2.3).
    let notified = friend::identity_rotation::notify_identity_rotated(host, &new_identity).await;
    info!(notified, "rotation notifications sent");

    Ok(RotatedIdentity {
        new_public_key_hex,
        friends_notified: u32::try_from(notified).unwrap_or(u32::MAX),
    })
}

pub struct RotatedIdentity {
    pub new_public_key_hex: String,
    pub friends_notified: u32,
}

//! Identity lifecycle operations — rotate, destroy. An identity's records
//! are created by the unlock's own-records stage (`rekindle_node::host`).
//!
//! Orchestration logic that composes:
//! - `broadcast::dht_writes` for raw DHT primitives (create, open, set, close)
//! - `broadcast::dm` for DM sends (rotation notifications)
//! - `dht/*` typed modules for business logic reads/writes (profile, mailbox, friend list)

use tracing::info;

use crate::broadcast::node::TransportNode;
use crate::error::Result;

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

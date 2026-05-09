//! Direct message operations — send via DhtLog, read inbox.
//!
//! All sends route through `broadcast::dm` and `broadcast::dht_writes`.
//! Keypair bytes are deserialized inside the broadcast boundary.

use crate::error::Result;
use crate::broadcast::node::TransportNode;
use crate::session::Session;

/// Send a direct message to a peer via their outbound DhtLog.
///
/// `body` is Signal-encrypted ciphertext bytes. The caller (handle_dm_send
/// in the node crate) performs Signal encrypt before calling this.
///
/// `dm_log_keypair_bytes`: 64-byte serialized keypair for the outbound DhtLog.
pub async fn send_dm(
    node: &TransportNode,
    session: &Session,
    peer_key: &str,
    body: &[u8],
    signing_key_bytes: &[u8; 32],
    dm_log_keypair_bytes: Option<&[u8]>,
) -> Result<()> {
    let dm_log_keypair = dm_log_keypair_bytes
        .map(crate::broadcast::node::deserialize_keypair)
        .transpose()?;
    crate::broadcast::dm::direct_message(
        node, session, peer_key, body, None,
        signing_key_bytes, dm_log_keypair,
    ).await
}

/// Send a typing indicator to a peer (always app_message — ephemeral).
pub async fn send_typing(
    node: &TransportNode,
    session: &Session,
    peer_key: &str,
    typing: bool,
    signing_key_bytes: &[u8; 32],
) -> Result<()> {
    crate::broadcast::dm::typing(node, session, peer_key, typing, signing_key_bytes).await
}

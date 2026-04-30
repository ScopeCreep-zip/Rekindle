//! Direct message operations — send, read inbox.

use tracing::info;

use crate::error::{TransportError, Result};
use crate::node::TransportNode;
use crate::payload::dm::{DmPayload, dm_type_id, serialize_dm};
use crate::session::Session;

/// Send a direct message to a peer.
///
/// Steps:
/// 1. Resolve the peer's route (from peer registry or mailbox DHT)
/// 2. Build `DmPayload::DirectMessage` with the message body
/// 3. Send via `Sender::send_dm` (Ed25519 signed envelope)
///
/// The body is sent as signed plaintext within the Ed25519 `SignedPayload`
/// envelope. End-to-end Signal Protocol encryption is applied at the
/// application layer by the CLI/TUI after establishing a session via the
/// friend request/accept handshake. The transport layer guarantees
/// authentication (signature verification) and route-level privacy
/// (Veilid safety routing).
pub async fn send_dm(
    node: &TransportNode,
    session: &Session,
    peer_key: &str,
    body: &str,
    signing_key_bytes: &[u8; 32],
) -> Result<()> {
    info!(peer = &peer_key[..12], "sending DM");

    // Resolve peer route
    let route_blob = {
        let peers = node.peers();
        let registry = peers.read();
        registry.get_route(peer_key).map(<[u8]>::to_vec)
    };

    let route_blob = if let Some(blob) = route_blob {
        blob
    } else {
        // Try to read from peer's mailbox DHT
        let dht = node.dht()?;
        dht.mailbox()
            .read_peer_route(peer_key)
            .await?
            .ok_or_else(|| TransportError::NoRoute {
                peer: peer_key.to_string(),
            })?
    };

    let target = node.import_route(&route_blob)?;

    // Build DM payload
    let dm = DmPayload::DirectMessage {
        body: body.as_bytes().to_vec(),
        reply_to: None,
    };

    let payload_bytes = serialize_dm(&dm)?;
    let type_id = dm_type_id(&dm);

    // Send
    node.sender()
        .send_dm(
            &target,
            type_id,
            signing_key_bytes,
            &session.identity.public_key_hex,
            &payload_bytes,
        )
        .await?;

    info!(peer = &peer_key[..12], "DM sent");
    Ok(())
}

/// Send a typing indicator to a peer.
pub async fn send_typing(
    node: &TransportNode,
    session: &Session,
    peer_key: &str,
    typing: bool,
    signing_key_bytes: &[u8; 32],
) -> Result<()> {
    let route_blob = {
        let peers = node.peers();
        let registry = peers.read();
        registry.get_route(peer_key).map(<[u8]>::to_vec)
    };

    let route_blob = route_blob.ok_or_else(|| TransportError::NoRoute {
        peer: peer_key.to_string(),
    })?;

    let target = node.import_route(&route_blob)?;

    let dm = DmPayload::Typing { typing };
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
        .await
}

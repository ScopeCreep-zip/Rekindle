//! A live 1:1 call's roster follows its peer's route (→ E4.1, which moves
//! the call FSM and this heal into the Host). Call signaling carries no
//! route blobs, so the one peer-route cache (`PeerRegistry`, plan E2.1) —
//! fed by the friend's profile watch, their mailbox and the handshake —
//! is the only source of fresh routes for DM call media.

use std::sync::Arc;

use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::state::AppState;

/// Push each peer's new route into their slot of a live 1:1 call's roster,
/// so frames survive the peer's route rotation; until `stop`.
pub async fn heal_call_routes(
    state: Arc<AppState>,
    node: Arc<rekindle_transport::TransportNode>,
    mut updates: broadcast::Receiver<String>,
    stop: CancellationToken,
) {
    loop {
        let peer = tokio::select! {
            () = stop.cancelled() => return,
            next = updates.recv() => next,
        };
        let peer = match peer {
            Ok(peer) => peer,
            // Missed changes are only missed heals: the next one carries
            // the current route.
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => return,
        };
        let in_call = state
            .active_calls
            .list_all()
            .iter()
            .any(|c| c.peer_pubkey == peer);
        if !in_call {
            continue;
        }
        let Some(blob) = rekindle_transport::operations::dm::route::cached(&node, &peer) else {
            continue;
        };
        let transport = {
            let ve = state.voice_engine.lock();
            ve.as_ref()
                .filter(|h| h.community_id.is_none())
                .map(|h| h.transport.clone())
        };
        if let Some(transport) = transport {
            transport.lock().await.refresh_peer_route(&peer, &blob);
        }
    }
}

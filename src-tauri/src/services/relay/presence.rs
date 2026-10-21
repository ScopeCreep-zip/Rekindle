//! Strand Relay presence caching (architecture §13.5), Carol's side: when
//! she relays for Bob, she serves Bob's status to friends asking "do you
//! know Bob?" — faster than a DHT lookup (→ E5.2). Alice's probe and her
//! handling of the answer are the transport's (`operations::relay`, plan
//! E2.1).

use std::sync::Arc;

use rekindle_codec::capnp_codec::direct::{DirectBody, StatusResponse};

use crate::db_helpers::db_call;
use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;
use rusqlite::OptionalExtension;

/// Carol's side: respond to a `StatusRequest` (0x0140) from `requester` asking
/// about `target`. Replies only when:
///   - the requester is a known friend (privacy: don't gossip our
///     friends' status to strangers), and
///   - we have an active `strand_relay_volunteered` row for `target`.
pub async fn respond_to_status_request(
    state: &Arc<AppState>,
    pool: &Db,
    requester_pubkey: &str,
    target_pubkey: &str,
) -> Result<(), String> {
    if !state_helpers::is_friend(state, requester_pubkey) {
        return Err("status request from non-friend".into());
    }

    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return Err("no identity".into());
    }
    let owner = owner_key;
    let target = target_pubkey.to_string();
    let volunteered: bool = db_call(pool, move |conn| {
        Ok(conn
            .query_row(
                "SELECT 1 FROM strand_relay_volunteered
                 WHERE owner_key = ?1 AND friend_public_key = ?2 LIMIT 1",
                rusqlite::params![owner, target],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    })
    .await?;
    if !volunteered {
        // Architecture §13.5: only relay friends serve a peer's status.
        // Reply with empty so the requester can fall back to DHT.
        let body = DirectBody::StatusResponse(StatusResponse {
            target_pubkey: target_pubkey.to_string(),
            status: String::new(),
            status_message: None,
            last_seen: 0,
            route_blob: Vec::new(),
        });
        return send(state, requester_pubkey, &body).await;
    }

    // Our snapshot of the friend, from the Host's projection.
    let snapshot = state_helpers::friend_field(state, target_pubkey, |f| {
        let last_seen = f
            .last_heartbeat_at
            .or(f.last_seen_at)
            .map_or(0, |ms| u64::try_from(ms / 1000).unwrap_or(0));
        Some((
            f.status.as_wire_str().to_string(),
            f.status_message.clone(),
            last_seen,
        ))
    });
    let route_blob = state_helpers::peer_route(state, target_pubkey).unwrap_or_default();

    let (status, status_message, last_seen) =
        snapshot.unwrap_or_else(|| (String::from("offline"), None, 0));
    let body = DirectBody::StatusResponse(StatusResponse {
        target_pubkey: target_pubkey.to_string(),
        status,
        status_message,
        last_seen,
        route_blob,
    });
    send(state, requester_pubkey, &body).await
}

/// Send through the Host's 1:1 path (plan E2.1), ratchet-sealed (0x0141).
async fn send(state: &AppState, to: &str, body: &DirectBody) -> Result<(), String> {
    rekindle_transport::operations::dm::send_to_peer(
        state_helpers::friend_host(state)?.as_ref(),
        to,
        body,
    )
    .await
    .map_err(|e| e.to_string())
}

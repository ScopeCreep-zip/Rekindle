//! Social dispatch handlers: Friend*, Dm*.

use std::sync::Arc;

use crate::daemon::DaemonState;
use rekindle_ipc::protocol::IpcResponse;

use super::{state_error, store_error, DaemonContext};

pub(crate) async fn handle_friend_add(
    ctx: &DaemonContext,
    state: DaemonState,
    target: &str,
    message: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let signing_key = match ctx.require_signing_key() {
        Ok(k) => k,
        Err(e) => return e,
    };
    let session = match ctx.require_session(Clone::clone) {
        Ok(s) => s,
        Err(e) => return e,
    };
    // A bundle for this one peer, from the unlock's manager: its one-time
    // keys' private halves are in the vault before it is sent.
    let bundle = match ctx.require_signal().and_then(|signal| {
        signal
            .handout_bundle()
            .map_err(|e| store_error("prekey bundle", e))
    }) {
        Ok(b) => b.to_wire(),
        Err(e) => return e,
    };

    match rekindle_transport::operations::friend::send_friend_request(
        &transport,
        &session,
        target,
        message,
        &signing_key,
        bundle,
    )
    .await
    {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "status": "sent", "target": target })),
        Err(e) => IpcResponse::error(500, format!("friend request failed: {e}")),
    }
}

pub(crate) async fn handle_friend_accept(
    ctx: &DaemonContext,
    state: DaemonState,
    public_key: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };

    let (session, request) = {
        // Check session for existing pending request first (no async needed)
        let found = {
            let guard = ctx.session.read();
            let Some(session) = guard.as_ref() else {
                return IpcResponse::error(404, "no identity loaded");
            };
            session
                .pending_request_by_key(public_key)
                .map(|req| (session.clone(), req.clone()))
        };
        if let Some(result) = found {
            result
        } else {
            // Not in session yet — actively scan the friend inbox.
            // The background poll may not have run yet, so we trigger a direct
            // scan with retry rather than passively waiting 60s.
            let inbox_key = ctx
                .session
                .read()
                .as_ref()
                .map(|s| s.identity.friend_inbox_key.clone());

            let scan_start = std::time::Instant::now();
            let scan_deadline = std::time::Duration::from_secs(30);
            let mut attempt = 0u32;

            if let Some(ref key) = inbox_key {
                tracing::info!(
                    public_key = %public_key,
                    "friend accept: request not in session, scanning inbox directly"
                );

                while scan_start.elapsed() < scan_deadline {
                    crate::daemon::friend_inbox::scan_friend_inbox(
                        &ctx.session,
                        &ctx.transport,
                        &ctx.db,
                        key,
                    )
                    .await;

                    let found = ctx
                        .session
                        .read()
                        .as_ref()
                        .and_then(|s| s.pending_request_by_key(public_key).cloned())
                        .is_some();
                    if found {
                        tracing::info!(
                            public_key = %public_key,
                            elapsed_secs = scan_start.elapsed().as_secs(),
                            attempt,
                            "friend accept: request discovered via direct scan"
                        );
                        break;
                    }

                    attempt += 1;
                    // Backoff between scans: 2s, 4s, 8s
                    let wait = std::time::Duration::from_secs(2u64.saturating_pow(attempt).min(8));
                    tokio::time::sleep(wait).await;
                }
            }

            // Re-read session after scan
            let guard = ctx.session.read();
            let Some(session) = guard.as_ref() else {
                return IpcResponse::error(404, "no identity loaded");
            };
            let Some(req) = session.pending_request_by_key(public_key) else {
                return IpcResponse::error(404, format!(
                    "no pending request from {} — they may not have sent one, or DHT propagation is still in progress",
                    rekindle_utils::text::prefix(public_key, 16),
                ));
            };
            (session.clone(), req.clone())
        }
    };

    match rekindle_transport::operations::friend::accept_friend_request(
        &transport,
        &session,
        &request.public_key,
        &request.route_blob,
        &request.profile_dht_key,
        &request.display_name,
    )
    .await
    {
        Ok(accepted) => {
            if let Err(e) = record_accept(ctx, &request, &accepted).await {
                return e;
            }
            IpcResponse::ok(&serde_json::json!({
                "accepted": public_key,
                "dm_log_key": accepted.dm_log_key,
            }))
        }
        Err(e) => IpcResponse::error(500, format!("friend accept failed: {e}")),
    }
}

pub(crate) async fn handle_friend_reject(
    ctx: &DaemonContext,
    state: DaemonState,
    public_key: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };

    let session = match ctx.require_session(Clone::clone) {
        Ok(s) => s,
        Err(e) => return e,
    };

    match rekindle_transport::operations::friend::reject_friend_request(
        &transport, &session, public_key,
    )
    .await
    {
        Ok(()) => {
            let owner = session.identity.public_key_hex.clone();
            let peer = public_key.to_string();
            let forgotten = match ctx.require_db() {
                Ok(db) => db
                    .call(move |conn| {
                        rekindle_db::repo::pending_requests::delete(conn, &owner, &peer)
                    })
                    .await
                    .map_err(|e| store_error("pending request", e)),
                Err(e) => Err(e),
            };
            if let Err(e) = forgotten {
                return e;
            }
            if let Some(s) = ctx.session.write().as_mut() {
                s.remove_pending_friend_request(public_key);
            }
            IpcResponse::ok(&serde_json::json!({ "rejected": public_key }))
        }
        Err(e) => IpcResponse::error(500, format!("friend reject failed: {e}")),
    }
}

pub(crate) async fn handle_friend_remove(
    ctx: &DaemonContext,
    state: DaemonState,
    public_key: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let session = match ctx.require_session(Clone::clone) {
        Ok(s) => s,
        Err(e) => return e,
    };

    match rekindle_transport::operations::friend::remove_friend(&transport, &session, public_key)
        .await
    {
        Ok(()) => {
            // The DM log watch with this peer goes with the friendship.
            let watch_leases = ctx
                .subscriptions
                .read()
                .as_ref()
                .map(|subs| subs.teardown_dm_peer(public_key))
                .unwrap_or_default();
            for lease in watch_leases {
                rekindle_transport::broadcast::dht_writes::release(&transport, lease).await;
            }
            IpcResponse::ok(&serde_json::json!({ "removed": public_key }))
        }
        Err(e) => IpcResponse::error(500, format!("friend remove failed: {e}")),
    }
}

pub(crate) async fn handle_friend_list(ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let (friend_list_key, our_key) = match ctx.require_session(|s| {
        (
            s.identity.friend_list_dht_key.clone(),
            s.identity.public_key_hex.clone(),
        )
    }) {
        Ok(t) => t,
        Err(e) => return e,
    };
    let query = match transport.query(Arc::clone(&ctx.mek_cache)) {
        Ok(q) => q,
        Err(e) => return IpcResponse::error(500, format!("query engine: {e}")),
    };
    match query.resolved_friends(&friend_list_key).await {
        Ok(friends) => {
            // Filter out the local user's own key (defensive — shouldn't be in the list
            // but can appear due to bidirectional friend request flows)
            let filtered: Vec<_> = friends
                .into_iter()
                .filter(|f| f.public_key != our_key)
                .collect();
            IpcResponse::ok(&filtered)
        }
        Err(e) => IpcResponse::error(500, format!("friend list: {e}")),
    }
}

pub(crate) fn handle_friend_requests(ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    ctx.require_session(|session| IpcResponse::ok(&session.pending_friend_requests))
        .unwrap_or_else(|e| e)
}

pub(crate) async fn handle_dm_send(
    ctx: &DaemonContext,
    state: DaemonState,
    peer_key: &str,
    body: &str,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let signing_key = match ctx.require_signing_key() {
        Ok(k) => k,
        Err(e) => return e,
    };
    let session = match ctx.require_session(Clone::clone) {
        Ok(s) => s,
        Err(e) => return e,
    };

    // DMs require friendship. Resolve mailbox from pending request or
    // dm_log_keys (populated during friend accept). If neither exists,
    // the peer isn't a friend and DMs are refused.
    let mailbox_key = if let Some(req) = session.pending_request_by_key(peer_key) {
        req.mailbox_dht_key.clone()
    } else if session.dm_log_keys.contains_key(peer_key) {
        // Has DhtLog — mailbox not needed for the DhtLog path
        String::new()
    } else {
        return IpcResponse::error(
            404,
            format!(
                "cannot DM '{}'  — not a friend. accept their request or send them one first",
                rekindle_utils::text::prefix(peer_key, 16),
            ),
        );
    };

    // The shared DhtLog's keypair, from the vault, when we have one with
    // this peer.
    let dm_log_keypair_bytes = if session.dm_log_keys.contains_key(peer_key) {
        let guard = ctx.vault.lock();
        let Some(vault) = guard.as_ref() else {
            return IpcResponse::error(423, "no identity is unlocked");
        };
        match rekindle_vault::typed::dm_log::load_dm_log_keypair(vault, peer_key) {
            Ok(found) => found.map(|bytes| bytes.to_vec()),
            Err(e) => return store_error("DM log keypair", e),
        }
    } else {
        None
    };

    match rekindle_transport::operations::dm::send_dm(
        &transport,
        &session,
        peer_key,
        &mailbox_key,
        body,
        &signing_key,
        dm_log_keypair_bytes.as_deref(),
    )
    .await
    {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "status": "sent", "peer_key": peer_key })),
        Err(e) => IpcResponse::error(500, format!("dm send failed: {e}")),
    }
}

pub(crate) async fn handle_dm_typing(
    ctx: &DaemonContext,
    state: DaemonState,
    peer_key: &str,
    typing: bool,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let signing_key = match ctx.require_signing_key() {
        Ok(k) => k,
        Err(e) => return e,
    };
    let session = match ctx.require_session(Clone::clone) {
        Ok(s) => s,
        Err(e) => return e,
    };

    match rekindle_transport::operations::dm::send_typing(
        &transport,
        &session,
        peer_key,
        typing,
        &signing_key,
    )
    .await
    {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "typing": typing })),
        Err(e) => IpcResponse::error(500, format!("typing indicator failed: {e}")),
    }
}

pub(crate) async fn handle_dm_inbox(
    ctx: &DaemonContext,
    state: DaemonState,
    limit: u32,
) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let transport = match ctx.require_transport() {
        Ok(t) => t,
        Err(e) => return e,
    };
    let (dm_log_keys, friend_list_key, our_key) = match ctx.require_session(|s| {
        (
            s.dm_log_keys.clone(),
            s.identity.friend_list_dht_key.clone(),
            s.identity.public_key_hex.clone(),
        )
    }) {
        Ok(t) => t,
        Err(e) => return e,
    };

    if dm_log_keys.is_empty() {
        return IpcResponse::ok(&serde_json::json!([]));
    }

    // Read each per-peer DhtLog and aggregate into threads
    let mut all_threads = Vec::new();
    let query = match transport.query(Arc::clone(&ctx.mek_cache)) {
        Ok(q) => q,
        Err(e) => return IpcResponse::error(500, format!("query engine: {e}")),
    };

    for (peer_key, log_key) in &dm_log_keys {
        match query
            .dm_inbox(log_key, &friend_list_key, limit as usize, &our_key)
            .await
        {
            Ok(mut threads) => all_threads.append(&mut threads),
            Err(e) => {
                tracing::debug!(peer = %peer_key, error = %e, "DM log read failed");
            }
        }
    }

    // Sort by most recent message first
    all_threads.sort_by(|a: &rekindle_transport::query::DmThreadDisplay, b| {
        b.last_message_at.cmp(&a.last_message_at)
    });

    IpcResponse::ok(&all_threads)
}

/// An accepted request, recorded: the DM log keypair into the vault, then
/// in one transaction the friend's row with its DM log key and the pending
/// request removed; then the projection.
async fn record_accept(
    ctx: &DaemonContext,
    request: &rekindle_transport::PendingFriendRequest,
    accepted: &rekindle_transport::operations::friend::FriendAccepted,
) -> Result<(), IpcResponse> {
    {
        let guard = ctx.vault.lock();
        let vault = guard
            .as_ref()
            .ok_or_else(|| IpcResponse::error(423, "no identity is unlocked"))?;
        rekindle_vault::typed::dm_log::persist_dm_log_keypair(
            vault,
            &request.public_key,
            &accepted.dm_log_keypair_bytes,
        )
        .map_err(|e| store_error("DM log keypair", e))?;
    }
    let owner = ctx.require_owner()?;
    let (peer, name, log_key) = (
        request.public_key.clone(),
        request.display_name.clone(),
        accepted.dm_log_key.clone(),
    );
    let now = rekindle_utils::timestamp_ms_i64();
    ctx.require_db()?
        .call(move |conn| {
            let tx = conn.transaction()?;
            rekindle_db::repo::friends::insert_accepted(&tx, &owner, &peer, &name, now)?;
            rekindle_db::repo::friends::set(
                &tx,
                &owner,
                &peer,
                rekindle_db::repo::friends::Column::DmLogKey,
                &log_key,
            )?;
            rekindle_db::repo::pending_requests::delete(&tx, &owner, &peer)?;
            tx.commit()
        })
        .await
        .map_err(|e| store_error("accepted friend", e))?;
    if let Some(s) = ctx.session.write().as_mut() {
        s.remove_pending_friend_request(&request.public_key);
        s.dm_log_keys
            .insert(request.public_key.clone(), accepted.dm_log_key.clone());
    }
    Ok(())
}

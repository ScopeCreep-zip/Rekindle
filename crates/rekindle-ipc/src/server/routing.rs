//! Frame routing: request-response correlation and pub-sub broadcast.

use uuid::Uuid;

use super::{ServerState, DAEMON_AGENT_NAME, RATE_LIMIT_MAX_TOKENS, RATE_LIMIT_REFILL_MS};

use crate::framing::{decode_frame, encode_frame};
use crate::message::{Message, WIRE_VERSION};
use crate::protocol::BusPayload;

/// Route a received frame to the appropriate destination(s).
///
/// Pure router — the server has zero knowledge of IPC request/response
/// semantics. It decodes the `Message<BusPayload>` envelope for routing
/// metadata (correlation_id, sender, security_level, verified_sender_name)
/// and forwards the frame to the correct destination(s).
///
/// - If `correlation_id` is set: this is a response — route to the
///   connection that originated the matching request.
/// - If `correlation_id` is None: this is a new request or broadcast —
///   record the `msg_id` for response routing and forward to all other
///   connections at sufficient clearance.
pub(super) async fn route_frame(state: &ServerState, sender_conn_id: u64, payload: &[u8]) {
    // Decode the message envelope. Single type for all bus traffic.
    let mut msg: Message<BusPayload> = match decode_frame(payload) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(conn_id = sender_conn_id, error = %e, "malformed frame, dropping");
            return;
        }
    };
    if msg.wire_version != WIRE_VERSION {
        tracing::warn!(
            conn_id = sender_conn_id,
            got = msg.wire_version,
            expected = WIRE_VERSION,
            "wire version mismatch, dropping"
        );
        return;
    }

    // Rate limit check: token bucket per connection.
    //
    // Daemon→client media frames BYPASS this limit: the token bucket bounds a
    // CLIENT that sends frames (video-media-engine plan Step 3), not the
    // daemon's video fan-out, which arrives at frame rate across many streams.
    // A non-daemon `Media` frame is dropped later in the `Media` arm, so
    // exempting it here cannot let an untrusted client flood the bus.
    {
        let conns = state.connections.read().await;
        if let Some(conn) = conns.get(&sender_conn_id) {
            let is_daemon_media = matches!(msg.payload, BusPayload::Media(_))
                && conn.verified_name.as_deref() == Some(DAEMON_AGENT_NAME);
            if !is_daemon_media {
                let now_ms = rekindle_utils::timestamp_ms();
                let last = conn
                    .last_token_refill
                    .load(std::sync::atomic::Ordering::Relaxed);
                if now_ms.saturating_sub(last) >= RATE_LIMIT_REFILL_MS {
                    conn.rate_tokens
                        .store(RATE_LIMIT_MAX_TOKENS, std::sync::atomic::Ordering::Relaxed);
                    conn.last_token_refill
                        .store(now_ms, std::sync::atomic::Ordering::Relaxed);
                }
                let prev = conn.rate_tokens.fetch_update(
                    std::sync::atomic::Ordering::Relaxed,
                    std::sync::atomic::Ordering::Relaxed,
                    |t| if t > 0 { Some(t - 1) } else { None },
                );
                if prev.is_err() {
                    tracing::warn!(
                        conn_id = sender_conn_id,
                        "rate limit exceeded, dropping frame"
                    );
                    return;
                }
            }
        }
    }

    // Sender identity verification and clearance enforcement.
    {
        let mut conns = state.connections.write().await;
        if let Some(conn) = conns.get_mut(&sender_conn_id) {
            // Identity immutability: once set, agent_id cannot change mid-session.
            if let Some(known_id) = conn.agent_id {
                if known_id != msg.sender {
                    tracing::warn!(
                        conn_id = sender_conn_id,
                        expected = %known_id,
                        got = %msg.sender,
                        "agent identity changed mid-session, dropping frame"
                    );
                    return;
                }
            } else {
                conn.agent_id = Some(msg.sender);
            }

            // [RC-6] Sender clearance enforcement.
            if conn.security_clearance < msg.security_level {
                tracing::warn!(
                    conn_id = sender_conn_id,
                    sender = ?conn.security_clearance,
                    msg = ?msg.security_level,
                    "clearance insufficient, rejecting"
                );
                return;
            }
        }
    }

    // Stamp the sender's verified name and Noise key from connection
    // state, overwriting whatever the client put there.
    {
        let conns = state.connections.read().await;
        let conn = conns.get(&sender_conn_id);
        msg.verified_sender_name = conn.and_then(|c| c.verified_name.clone());
        msg.verified_sender_key = conn.map(|c| c.static_key);
    }

    // Re-encode with server stamps applied.
    let stamped_payload = match encode_frame(&msg) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(conn_id = sender_conn_id, error = %e, "re-encode failed");
            return;
        }
    };

    // ── Fail-closed routing ──────────────────────────────────────────
    //
    // Messages are NEVER broadcast to all connections. Every message is
    // either a correlated response (unicast to originator), a request
    // (unicast to the daemon), or an event (subscription-filtered).
    // Anything that doesn't match a known routing path is dropped.

    if let Some(corr_id) = msg.correlation_id {
        // Correlated response: route to the connection that sent the
        // original request. Only the daemon answers requests; a correlated
        // frame from anyone else would let a client forge another
        // client's response.
        if !is_daemon(state, sender_conn_id).await {
            tracing::warn!(
                conn_id = sender_conn_id,
                "correlated frame from non-daemon source — rejected"
            );
            return;
        }
        let target_conn = state.pending_requests.write().await.remove(&corr_id);
        if let Some(target_id) = target_conn {
            let conns = state.connections.read().await;
            if let Some(target) = conns.get(&target_id) {
                if target.tx.try_send(stamped_payload).is_err() {
                    tracing::warn!(conn_id = target_id, "response dropped: channel full");
                }
            }
        } else {
            tracing::debug!(correlation_id = %corr_id, "response for unknown request, dropping");
        }
        return;
    }

    // No correlation_id — route based on payload type.
    match &msg.payload {
        BusPayload::Request(ref request) => {
            // Intercept Subscribe/Unsubscribe — handled server-side via EventRouter.
            // These never reach the daemon dispatch.
            match request {
                crate::protocol::IpcRequest::Subscribe { filters } => {
                    let sender_tx = {
                        let conns = state.connections.read().await;
                        conns.get(&sender_conn_id).map(|c| c.tx.clone())
                    };
                    let response = if let Some(tx) = sender_tx {
                        match state
                            .event_router
                            .write()
                            .subscribe(sender_conn_id, filters, tx)
                        {
                            Ok(count) => crate::protocol::IpcResponse::ok(&serde_json::json!({
                                "subscribed": true, "filter_count": count,
                            })),
                            Err(reason) => crate::protocol::IpcResponse::error(400, reason),
                        }
                    } else {
                        crate::protocol::IpcResponse::error(500, "connection state not found")
                    };
                    reply_direct(state, sender_conn_id, &msg, &response).await;
                }
                crate::protocol::IpcRequest::Unsubscribe { filters } => {
                    let remaining = state
                        .event_router
                        .write()
                        .unsubscribe(sender_conn_id, filters);
                    let response = crate::protocol::IpcResponse::ok(&serde_json::json!({
                        "unsubscribed": true, "remaining": remaining,
                    }));
                    reply_direct(state, sender_conn_id, &msg, &response).await;
                }
                _ => {
                    // All other requests: record for response routing, forward to daemon.
                    state
                        .pending_requests
                        .write()
                        .await
                        .insert(msg.msg_id, sender_conn_id);
                    let daemon_conn = state
                        .name_to_conn
                        .read()
                        .await
                        .get(DAEMON_AGENT_NAME)
                        .copied();
                    let refused = match daemon_conn {
                        // The daemon's own subscriber connects just after the
                        // bus binds; until then there is no one to answer.
                        None => Some("daemon starting — retry shortly"),
                        Some(daemon_id) => {
                            let conns = state.connections.read().await;
                            match conns.get(&daemon_id) {
                                Some(daemon) if daemon.tx.try_send(stamped_payload).is_ok() => None,
                                Some(_) => Some("daemon busy — retry shortly"),
                                None => Some("daemon starting — retry shortly"),
                            }
                        }
                    };
                    // Answer instead of dropping, so the client fails fast
                    // rather than waiting out its timeout.
                    if let Some(reason) = refused {
                        state.pending_requests.write().await.remove(&msg.msg_id);
                        tracing::warn!(conn_id = sender_conn_id, reason, "request not forwarded");
                        let response = crate::protocol::IpcResponse::error(503, reason);
                        reply_direct(state, sender_conn_id, &msg, &response).await;
                    }
                }
            }
        }
        BusPayload::Response(_) => {
            tracing::warn!(
                conn_id = sender_conn_id,
                "uncorrelated response without correlation_id, dropping"
            );
        }
        BusPayload::Event(ref event) => {
            // Events from the daemon's bridge task: route via EventRouter.
            // Only the daemon agent may send events. Other sources are rejected.
            if !is_daemon(state, sender_conn_id).await {
                tracing::warn!(
                    conn_id = sender_conn_id,
                    "event from non-daemon source — rejected"
                );
                return;
            }
            let (delivered, dropped) = state.event_router.read().deliver(event);
            tracing::debug!(delivered, dropped, "event routed via EventRouter");
        }
        BusPayload::Media(_) => {
            // Compressed video frame from the daemon. Fan out DIRECTLY to every
            // subscribed connection — bypassing the EventRouter's subscription
            // routing and each connection's ordered event queue, which exist for
            // events, not frame-rate media. Each connection has its own bounded, drop-oldest media
            // queue, so a slow client sheds stale frames without stalling the
            // rest or evicting queued control traffic.
            //
            // Daemon → client only: a client that sends `Media` is rejected
            // here (mirroring the `Event` arm), so a client can never inject
            // frames into another client's stream.
            if !is_daemon(state, sender_conn_id).await {
                tracing::warn!(
                    conn_id = sender_conn_id,
                    "media frame from non-daemon source — rejected"
                );
                return;
            }

            // Subscribers are the media audience: a video consumer subscribed
            // to the community's events. Fan out to all of them (noted: media
            // has no finer topic than the event subscription in Step 3), minus
            // the daemon's own connection so it never receives its own frames.
            let recipients = state.event_router.read().subscribed_conn_ids();
            let conns = state.connections.read().await;
            let mut delivered = 0usize;
            let mut evicted = 0usize;
            for conn_id in recipients {
                if conn_id == sender_conn_id {
                    continue;
                }
                if let Some(conn) = conns.get(&conn_id) {
                    if conn.media_tx.send(stamped_payload.clone()) {
                        evicted += 1;
                    }
                    delivered += 1;
                }
            }
            tracing::debug!(delivered, evicted, "media frame fanned out (drop-oldest)");
        }
    }
}

/// Whether `conn_id` is the daemon's own connection.
async fn is_daemon(state: &ServerState, conn_id: u64) -> bool {
    state
        .connections
        .read()
        .await
        .get(&conn_id)
        .and_then(|c| c.verified_name.as_deref())
        == Some(DAEMON_AGENT_NAME)
}

/// Answer `request` on its sender's connection directly, for the requests
/// the server handles itself.
async fn reply_direct(
    state: &ServerState,
    conn_id: u64,
    request: &Message<BusPayload>,
    response: &crate::protocol::IpcResponse,
) {
    let body = match serde_json::to_vec(response) {
        Ok(body) => body,
        Err(e) => {
            tracing::error!(conn_id, error = %e, "reply serialization failed");
            return;
        }
    };
    let reply = Message {
        wire_version: WIRE_VERSION,
        msg_id: request.msg_id,
        sender: Uuid::nil(),
        correlation_id: Some(request.msg_id),
        timestamp: crate::message::Timestamp::now(state.epoch),
        security_level: request.security_level,
        verified_sender_name: Some(DAEMON_AGENT_NAME.to_string()),
        verified_sender_key: None,
        agent_type: None,
        community_scope: None,
        payload: BusPayload::Response(body),
    };
    match encode_frame(&reply) {
        Ok(bytes) => {
            if let Some(conn) = state.connections.read().await.get(&conn_id) {
                if conn.tx.try_send(bytes).is_err() {
                    tracing::warn!(conn_id, "reply dropped: channel full");
                }
            }
        }
        Err(e) => tracing::error!(conn_id, error = %e, "reply encode failed"),
    }
}

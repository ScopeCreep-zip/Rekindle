//! Frame routing: request-response correlation and pub-sub broadcast.

use uuid::Uuid;

use super::{ServerState, DAEMON_AGENT_NAME, RATE_LIMIT_MAX_TOKENS, RATE_LIMIT_REFILL_MS};

use crate::ipc::framing::{decode_frame, encode_frame};
use crate::ipc::message::Message;
use crate::ipc::protocol::BusPayload;

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

    // Stamp verified_sender_name from connection state.
    {
        let conns = state.connections.read().await;
        if let Some(conn) = conns.get(&sender_conn_id) {
            msg.verified_sender_name.clone_from(&conn.verified_name);
        }
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
        // Correlated response: route to the connection that sent the original request.
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
                crate::ipc::protocol::IpcRequest::Subscribe { filters } => {
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
                            Ok(count) => {
                                crate::ipc::protocol::IpcResponse::ok(&serde_json::json!({
                                    "subscribed": true, "filter_count": count,
                                }))
                            }
                            Err(reason) => crate::ipc::protocol::IpcResponse::error(400, reason),
                        }
                    } else {
                        crate::ipc::protocol::IpcResponse::error(500, "connection state not found")
                    };
                    // Send response directly back to sender
                    let resp_msg = Message {
                        wire_version: crate::ipc::message::WIRE_VERSION,
                        msg_id: msg.msg_id,
                        sender: Uuid::nil(),
                        correlation_id: Some(msg.msg_id),
                        timestamp: crate::ipc::message::Timestamp::now(state.epoch),
                        security_level: msg.security_level,
                        verified_sender_name: Some(DAEMON_AGENT_NAME.to_string()),
                        agent_type: None,
                        community_scope: None,
                        payload: BusPayload::Response(
                            serde_json::to_vec(&response).unwrap_or_else(|_| b"{}".to_vec()),
                        ),
                    };
                    if let Ok(bytes) = encode_frame(&resp_msg) {
                        let conns = state.connections.read().await;
                        if let Some(conn) = conns.get(&sender_conn_id) {
                            let _ = conn.tx.try_send(bytes);
                        }
                    }
                }
                crate::ipc::protocol::IpcRequest::Unsubscribe { filters } => {
                    let remaining = state
                        .event_router
                        .write()
                        .unsubscribe(sender_conn_id, filters);
                    let response = crate::ipc::protocol::IpcResponse::ok(&serde_json::json!({
                        "unsubscribed": true, "remaining": remaining,
                    }));
                    let resp_msg = Message {
                        wire_version: crate::ipc::message::WIRE_VERSION,
                        msg_id: msg.msg_id,
                        sender: Uuid::nil(),
                        correlation_id: Some(msg.msg_id),
                        timestamp: crate::ipc::message::Timestamp::now(state.epoch),
                        security_level: msg.security_level,
                        verified_sender_name: Some(DAEMON_AGENT_NAME.to_string()),
                        agent_type: None,
                        community_scope: None,
                        payload: BusPayload::Response(
                            serde_json::to_vec(&response).unwrap_or_else(|_| b"{}".to_vec()),
                        ),
                    };
                    if let Ok(bytes) = encode_frame(&resp_msg) {
                        let conns = state.connections.read().await;
                        if let Some(conn) = conns.get(&sender_conn_id) {
                            let _ = conn.tx.try_send(bytes);
                        }
                    }
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
                    if let Some(daemon_id) = daemon_conn {
                        let conns = state.connections.read().await;
                        if let Some(daemon) = conns.get(&daemon_id) {
                            if daemon.tx.try_send(stamped_payload).is_err() {
                                tracing::warn!("request dropped: daemon channel full");
                            }
                        }
                    } else {
                        tracing::error!("request dropped: daemon not connected to bus");
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
            let is_daemon = {
                let conns = state.connections.read().await;
                conns
                    .get(&sender_conn_id)
                    .and_then(|c| c.verified_name.as_deref())
                    == Some(DAEMON_AGENT_NAME)
            };
            if !is_daemon {
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
            // subscribed connection — bypassing the EventRouter's per-category
            // dedup and journal, which exist for ordered events, not frame-rate
            // media. Each connection has its own bounded, drop-oldest media
            // queue, so a slow client sheds stale frames without stalling the
            // rest or evicting queued control traffic.
            //
            // Daemon → client only: a client that sends `Media` is rejected
            // here (mirroring the `Event` arm), so a client can never inject
            // frames into another client's stream.
            let is_daemon = {
                let conns = state.connections.read().await;
                conns
                    .get(&sender_conn_id)
                    .and_then(|c| c.verified_name.as_deref())
                    == Some(DAEMON_AGENT_NAME)
            };
            if !is_daemon {
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

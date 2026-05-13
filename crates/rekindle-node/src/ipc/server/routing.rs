//! Frame routing: classifies and dispatches control-plane frames.
//!
//! Pure synchronous routing logic — takes a `&ServerState` and a `Bytes`
//! payload and routes it. No I/O, no async.
//!
//! Bulk frames never reach this module — they are dispatched by the
//! connection handler directly to the `BulkDispatcher` based on the
//! lane byte. This module handles only lane-0x00 (Noise-decrypted)
//! control-plane frames.

use std::sync::Arc;

use bytes::Bytes;
use uuid::Uuid;

use crate::ipc::framing::{decode_frame, encode_frame};
use crate::ipc::message::{Message, RoutingHeader, SecurityLevel, WIRE_VERSION};
use crate::ipc::protocol::BusPayload;

use super::constants::DAEMON_NAME_ARC;
use super::state::ServerState;

// ── Payload Classification ────────────────────────────────────────────

/// Discriminant of `BusPayload` — determines routing without full decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadKind {
    Request,
    Response,
    Event,
    Unknown,
}

/// Classify a `BusPayload` by reading the postcard varint discriminant.
#[inline]
fn classify_payload(remaining: &[u8]) -> PayloadKind {
    match remaining.first() {
        Some(&0) => PayloadKind::Request,
        Some(&1) => PayloadKind::Response,
        Some(&2) => PayloadKind::Event,
        _ => PayloadKind::Unknown,
    }
}

/// Postcard discriminant for `IpcRequest::Subscribe`.
///
/// Postcard enum discriminants are positional indices (0-based).
/// Adding a variant BEFORE Subscribe in the IpcRequest enum changes
/// this value. When `test_subscribe_discriminants` fails after adding
/// a variant, update these constants to the values the test prints.
const DISCRIMINANT_SUBSCRIBE: u8 = 41;
/// Postcard discriminant for `IpcRequest::Unsubscribe`.
const DISCRIMINANT_UNSUBSCRIBE: u8 = 42;

/// Check if the IpcRequest discriminant is Subscribe or Unsubscribe.
#[inline]
fn is_subscribe_request(remaining: &[u8]) -> bool {
    matches!(
        remaining.get(1),
        Some(&DISCRIMINANT_SUBSCRIBE | &DISCRIMINANT_UNSUBSCRIBE)
    )
}

// ── Frame Routing ─────────────────────────────────────────────────────

/// Route a Noise-decrypted control frame to its destination.
///
/// This is the control-plane routing path. Bulk frames never reach here.
pub fn route_frame(state: &ServerState, sender_conn_id: u64, payload: Bytes) {
    // Phase 1: Parse routing header + classify payload (no locks).
    let (header, kind, needs_full_decode) = {
        let (h, remaining) = match postcard::take_from_bytes::<RoutingHeader>(&payload) {
            Ok((h, r)) => (h, r),
            Err(e) => {
                tracing::warn!(conn_id = sender_conn_id, error = %e, "malformed routing header");
                return;
            }
        };
        let kind = classify_payload(remaining);
        let needs_full_decode = kind == PayloadKind::Request && is_subscribe_request(remaining);
        (h, kind, needs_full_decode)
    };

    // Phase 2: Per-connection validation (single DashMap shard lock).
    {
        let Some(conn) = state.connections.get(&sender_conn_id) else {
            return;
        };

        if !conn.rate_limiter.try_consume() {
            tracing::warn!(conn_id = sender_conn_id, "rate limit exceeded");
            return;
        }

        let established_id = conn.agent_id.get_or_init(|| header.sender);
        if *established_id != header.sender {
            tracing::warn!(conn_id = sender_conn_id, "agent identity changed mid-session");
            return;
        }

        if conn.security_clearance < header.security_level {
            tracing::warn!(conn_id = sender_conn_id, "clearance insufficient");
            return;
        }
    }

    // Phase 3: Correlated response routing.
    if let Some(corr_id) = header.correlation_id {
        let target_conn = state.pending_requests.lock().remove_by_msg_id(&corr_id);
        if let Some(target_id) = target_conn {
            if let Some(target) = state.connections.get(&target_id) {
                if target.response_tx.try_send(payload.clone()).is_err() {
                    tracing::warn!(conn_id = target_id, "response dropped: channel full");
                }
            }
        } else {
            tracing::debug!(correlation_id = %corr_id, "response for unknown request");
        }
        return;
    }

    // Phase 4: Discriminant-based routing.
    match kind {
        PayloadKind::Request => {
            if needs_full_decode {
                route_subscribe_request(state, sender_conn_id, &header, &payload);
            } else {
                forward_to_daemon(state, &header, sender_conn_id, payload);
            }
        }
        PayloadKind::Response => {
            tracing::warn!(conn_id = sender_conn_id, "uncorrelated response without correlation_id");
        }
        PayloadKind::Event => {
            route_event(state, sender_conn_id, &payload);
        }
        PayloadKind::Unknown => {
            tracing::warn!(conn_id = sender_conn_id, "unknown payload discriminant — dropped");
        }
    }
}

/// Handle Subscribe/Unsubscribe requests (server-side, not forwarded to daemon).
fn route_subscribe_request(
    state: &ServerState,
    sender_conn_id: u64,
    header: &RoutingHeader,
    payload: &Bytes,
) {
    let msg: Message<BusPayload> = match decode_frame(payload) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(conn_id = sender_conn_id, error = %e, "request decode failed");
            return;
        }
    };
    let BusPayload::Request(ref request) = msg.payload else { return };
    match request {
        crate::ipc::protocol::IpcRequest::Subscribe { filters } => {
            let sender_tx = state.connections.get(&sender_conn_id).map(|c| c.event_tx.clone());
            let response = if let Some(tx) = sender_tx {
                match state.event_router.subscribe(sender_conn_id, filters, tx) {
                    Ok(count) => crate::ipc::protocol::IpcResponse::ok(&serde_json::json!({
                        "subscribed": true, "filter_count": count,
                    })),
                    Err(reason) => crate::ipc::protocol::IpcResponse::error(400, reason),
                }
            } else {
                crate::ipc::protocol::IpcResponse::error(500, "connection state not found")
            };
            send_response_to(state, sender_conn_id, header.msg_id, header.security_level, &response);
        }
        crate::ipc::protocol::IpcRequest::Unsubscribe { filters } => {
            let remaining = state.event_router.unsubscribe(sender_conn_id, filters);
            let response = crate::ipc::protocol::IpcResponse::ok(&serde_json::json!({
                "unsubscribed": true, "remaining": remaining,
            }));
            send_response_to(state, sender_conn_id, header.msg_id, header.security_level, &response);
        }
        _ => {
            forward_to_daemon(state, header, sender_conn_id, payload.clone());
        }
    }
}

/// Route an event from the daemon to subscribers via EventRouter.
fn route_event(state: &ServerState, sender_conn_id: u64, payload: &Bytes) {
    let is_daemon = state
        .connections
        .get(&sender_conn_id)
        .and_then(|c| c.verified_name.as_deref().map(|n| n == super::constants::DAEMON_AGENT_NAME))
        .unwrap_or(false);
    if !is_daemon {
        tracing::warn!(conn_id = sender_conn_id, "event from non-daemon source — rejected");
        return;
    }
    let msg: Message<BusPayload> = match decode_frame(payload) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(conn_id = sender_conn_id, error = %e, "event decode failed");
            return;
        }
    };
    if let BusPayload::Event(ref event) = msg.payload {
        let (delivered, dropped) = state.event_router.deliver(event);
        tracing::debug!(delivered, dropped, "event routed");
    }
}

/// Forward a request frame to the daemon subscriber as a `RoutedFrame`.
pub fn forward_to_daemon(
    state: &ServerState,
    header: &RoutingHeader,
    sender_conn_id: u64,
    payload: Bytes,
) {
    state.pending_requests.lock().insert(header.msg_id, sender_conn_id);

    let verified_name = state
        .connections
        .get(&sender_conn_id)
        .and_then(|c| c.verified_name.clone());

    let routed = crate::ipc::message::RoutedFrame {
        header: header.clone(),
        verified_sender_name: verified_name,
        raw: payload,
    };

    let daemon_tx = state.daemon_tx.read();
    if let Some(ref tx) = *daemon_tx {
        if tx.try_send(routed).is_err() {
            tracing::warn!("request dropped: daemon channel full");
        }
    } else {
        tracing::error!("request dropped: daemon subscriber not registered");
    }
}

/// Send a server-generated response directly to a connection.
pub fn send_response_to(
    state: &ServerState,
    conn_id: u64,
    request_msg_id: Uuid,
    level: SecurityLevel,
    response: &crate::ipc::protocol::IpcResponse,
) {
    let resp_msg = Message {
        wire_version: WIRE_VERSION,
        msg_id: request_msg_id,
        sender: Uuid::nil(),
        correlation_id: Some(request_msg_id),
        security_level: level,
        timestamp: crate::ipc::message::Timestamp::now(state.epoch),
        payload: BusPayload::Response(
            serde_json::to_vec(response).unwrap_or_else(|_| b"{}".to_vec()),
        ),
        verified_sender_name: Some(Arc::clone(&DAEMON_NAME_ARC)),
        agent_type: None,
        community_scope: None,
    };
    if let Ok(bytes) = encode_frame(&resp_msg) {
        if let Some(conn) = state.connections.get(&conn_id) {
            let _ = conn.response_tx.try_send(Bytes::from(bytes));
        }
    }
}

#[cfg(test)]
mod discriminant_tests {
    use super::*;

    #[test]
    fn test_subscribe_discriminants() {
        let subscribe = crate::ipc::protocol::IpcRequest::Subscribe {
            filters: vec![],
        };
        let unsub = crate::ipc::protocol::IpcRequest::Unsubscribe {
            filters: vec![],
        };

        let sub_bytes = postcard::to_allocvec(&subscribe).unwrap();
        let unsub_bytes = postcard::to_allocvec(&unsub).unwrap();

        assert_eq!(
            sub_bytes[0], DISCRIMINANT_SUBSCRIBE,
            "IpcRequest::Subscribe discriminant changed (was {}, now {})",
            DISCRIMINANT_SUBSCRIBE, sub_bytes[0]
        );
        assert_eq!(
            unsub_bytes[0], DISCRIMINANT_UNSUBSCRIBE,
            "IpcRequest::Unsubscribe discriminant changed (was {}, now {})",
            DISCRIMINANT_UNSUBSCRIBE, unsub_bytes[0]
        );
    }
}

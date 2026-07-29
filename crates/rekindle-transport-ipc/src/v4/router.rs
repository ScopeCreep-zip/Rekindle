//! FrameRouter — the single injection point between transport and application.
//!
//! The transport decodes, authenticates, and routes every frame. Application-visible
//! content is delivered through this trait. Implementations MUST be Send + Sync
//! and non-blocking — a blocking callback starves the connection's read loop.

use std::fmt;
use std::sync::Arc;

use crate::v4::wire::capability::CapabilityBits;
use crate::v4::wire::clearance::Clearance;

/// Metadata about the connection that produced a callback.
/// Passed to every FrameRouter method so the application can make
/// routing decisions without maintaining its own conn_id mapping.
#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    pub conn_id: u64,
    pub session_id: uuid::Uuid,
    pub peer_id: [u8; 32],
    pub clearance: Clearance,
    pub capabilities: CapabilityBits,
}

/// Application-visible connection lifecycle phase.
///
/// The transport fires `on_connection_state_change` with these variants.
/// The application matches exhaustively — no stringly-typed contracts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectionPhase {
    Handshaking,
    Established,
    Degraded,
    Draining,
    Dead,
    Closed,
}

impl fmt::Display for ConnectionPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Handshaking => write!(f, "Handshaking"),
            Self::Established => write!(f, "Established"),
            Self::Degraded => write!(f, "Degraded"),
            Self::Draining => write!(f, "Draining"),
            Self::Dead => write!(f, "Dead"),
            Self::Closed => write!(f, "Closed"),
        }
    }
}

/// Application-level frame routing. The transport calls these methods
/// after full authentication (EMAC, HeaderMAC, AEAD) and protocol-level
/// processing (audit chain, flow control, conditions evaluation).
///
/// Typed callbacks for each Datagram pattern eliminate redundant
/// application-side dispatch. `route_frame` is the fallback for
/// any frame the typed callbacks don't cover.
pub trait FrameRouter: Send + Sync + 'static {
    /// A decoded application frame that doesn't match a typed callback.
    /// `class` and `kind` are the FrameClass and FrameKind bytes.
    /// `payload` is the decrypted content after class+kind prefix.
    fn route_frame(
        &self,
        info: &ConnectionInfo,
        class: u8,
        kind: u8,
        payload: &[u8],
    );

    /// An RPC-style request. The application processes it and sends
    /// a DATAGRAM_REPLY with the same correlation message_id.
    fn on_request(
        &self,
        info: &ConnectionInfo,
        message_id: uuid::Uuid,
        sender_clearance: Clearance,
        payload: &[u8],
    );

    /// A reliable notification. Acknowledged at the protocol level;
    /// no semantic reply expected from the application.
    fn on_notify(
        &self,
        info: &ConnectionInfo,
        message_id: uuid::Uuid,
        sender_clearance: Clearance,
        payload: &[u8],
    );

    /// A pub-sub event that matched a subscription and passed conditions.
    fn on_publish(
        &self,
        info: &ConnectionInfo,
        subscription_id: uuid::Uuid,
        topic_hash: &[u8; 32],
        event_seq: u32,
        payload: &[u8],
    );

    /// A reply to a previously-sent DATAGRAM_REQUEST.
    fn on_reply(
        &self,
        info: &ConnectionInfo,
        message_id: uuid::Uuid,
        correlation_id: uuid::Uuid,
        status_phase: u32,
        payload: &[u8],
    );

    /// A DATAGRAM_REJECT for a previously-sent datagram.
    fn on_reject(
        &self,
        info: &ConnectionInfo,
        rejected_message_id: uuid::Uuid,
        reason_code: u32,
        detail: &str,
    );

    /// A bulk stream completed with verified content hash.
    /// Bulk data chunks are delivered directly through the bulk data channel
    /// (Vec<u8> bytes), NOT through the router. The router receives only
    /// this completion event with transfer metadata.
    fn on_bulk_complete(
        &self,
        info: &ConnectionInfo,
        stream_id: u8,
        transfer_id: uuid::Uuid,
        total_bytes: u64,
        total_chunks: u32,
    );

    /// A bulk stream failed after partial delivery.
    fn on_bulk_failed(
        &self,
        info: &ConnectionInfo,
        stream_id: u8,
        transfer_id: uuid::Uuid,
        reason: &str,
    );

    /// A single decrypted bulk data chunk received from the peer.
    /// Called for every chunk in delivery order (reassembler-ordered).
    /// The application accumulates these to reconstruct the payload.
    /// `on_bulk_complete` fires after all chunks for a stream are delivered.
    fn on_bulk_chunk(
        &self,
        info: &ConnectionInfo,
        stream_id: u8,
        chunk_index: u32,
        data: &[u8],
    );

    /// A batched acknowledgement of one or more message IDs.
    /// The client uses this to resolve pending send_request waiters.
    /// The server uses this for protocol-level bookkeeping.
    fn on_ack(
        &self,
        info: &ConnectionInfo,
        message_ids: &[uuid::Uuid],
    );

    /// Connection lifecycle state changed.
    fn on_connection_state_change(
        &self,
        info: &ConnectionInfo,
        old_phase: ConnectionPhase,
        new_phase: ConnectionPhase,
    );

    /// A shared-memory arena slot was published (Tier 2 streaming).
    /// `data` is a zero-copy slice into the mmap'd arena — do not hold
    /// past the callback return. The transport sends SlotRelease after
    /// this callback returns.
    fn on_arena_write(
        &self,
        info: &ConnectionInfo,
        shmref: &crate::v4::streaming::shared_arena::SharedMemRef,
        data: &[u8],
    );

    /// A DMA-BUF reference was received (Tier 3 GPU pass-through).
    /// The actual dmabuf fd was received via sidechannel and is available
    /// for GPU import. `payload_id` correlates with the sidechannel tag.
    fn on_dmabuf_ref(
        &self,
        info: &ConnectionInfo,
        dmabuf: &crate::v4::streaming::dmabuf::DmaBufRef,
        payload_id: u8,
    );
}

// ── MockRouter for tests ─────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CapturedRequest {
    pub info: ConnectionInfo,
    pub message_id: uuid::Uuid,
    pub sender_clearance: Clearance,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct CapturedNotify {
    pub info: ConnectionInfo,
    pub message_id: uuid::Uuid,
    pub sender_clearance: Clearance,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct CapturedPublish {
    pub info: ConnectionInfo,
    pub subscription_id: uuid::Uuid,
    pub topic_hash: [u8; 32],
    pub event_seq: u32,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct CapturedReply {
    pub info: ConnectionInfo,
    pub message_id: uuid::Uuid,
    pub correlation_id: uuid::Uuid,
    pub status_phase: u32,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct CapturedReject {
    pub info: ConnectionInfo,
    pub rejected_message_id: uuid::Uuid,
    pub reason_code: u32,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct CapturedBulkComplete {
    pub info: ConnectionInfo,
    pub stream_id: u8,
    pub transfer_id: uuid::Uuid,
    pub total_bytes: u64,
    pub total_chunks: u32,
}

#[derive(Debug, Clone)]
pub struct CapturedBulkFailed {
    pub info: ConnectionInfo,
    pub stream_id: u8,
    pub transfer_id: uuid::Uuid,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct CapturedBulkChunk {
    pub info: ConnectionInfo,
    pub stream_id: u8,
    pub chunk_index: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct CapturedRouteFrame {
    pub info: ConnectionInfo,
    pub class: u8,
    pub kind: u8,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct CapturedAck {
    pub info: ConnectionInfo,
    pub message_ids: Vec<uuid::Uuid>,
}

#[derive(Debug, Clone)]
pub struct CapturedStateChange {
    pub info: ConnectionInfo,
    pub old_phase: ConnectionPhase,
    pub new_phase: ConnectionPhase,
}

#[derive(Debug, Clone)]
pub struct CapturedArenaWrite {
    pub info: ConnectionInfo,
    pub shmref: crate::v4::streaming::shared_arena::SharedMemRef,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct CapturedDmaBufRef {
    pub info: ConnectionInfo,
    pub dmabuf: crate::v4::streaming::dmabuf::DmaBufRef,
    pub payload_id: u8,
}

/// Records every delivery for assertion in tests.
#[derive(Default)]
pub struct MockRouter {
    pub requests: parking_lot::Mutex<Vec<CapturedRequest>>,
    pub notifications: parking_lot::Mutex<Vec<CapturedNotify>>,
    pub publishes: parking_lot::Mutex<Vec<CapturedPublish>>,
    pub replies: parking_lot::Mutex<Vec<CapturedReply>>,
    pub rejects: parking_lot::Mutex<Vec<CapturedReject>>,
    pub bulk_completes: parking_lot::Mutex<Vec<CapturedBulkComplete>>,
    pub bulk_failures: parking_lot::Mutex<Vec<CapturedBulkFailed>>,
    pub bulk_chunks: parking_lot::Mutex<Vec<CapturedBulkChunk>>,
    pub acks: parking_lot::Mutex<Vec<CapturedAck>>,
    pub route_frames: parking_lot::Mutex<Vec<CapturedRouteFrame>>,
    pub state_changes: parking_lot::Mutex<Vec<CapturedStateChange>>,
    pub arena_writes: parking_lot::Mutex<Vec<CapturedArenaWrite>>,
    pub dmabuf_refs: parking_lot::Mutex<Vec<CapturedDmaBufRef>>,
}

impl MockRouter {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

impl FrameRouter for MockRouter {
    fn route_frame(&self, info: &ConnectionInfo, class: u8, kind: u8, payload: &[u8]) {
        self.route_frames.lock().push(CapturedRouteFrame {
            info: info.clone(), class, kind, payload: payload.to_vec(),
        });
    }

    fn on_request(&self, info: &ConnectionInfo, message_id: uuid::Uuid, sender_clearance: Clearance, payload: &[u8]) {
        self.requests.lock().push(CapturedRequest {
            info: info.clone(), message_id, sender_clearance, payload: payload.to_vec(),
        });
    }

    fn on_notify(&self, info: &ConnectionInfo, message_id: uuid::Uuid, sender_clearance: Clearance, payload: &[u8]) {
        self.notifications.lock().push(CapturedNotify {
            info: info.clone(), message_id, sender_clearance, payload: payload.to_vec(),
        });
    }

    fn on_publish(&self, info: &ConnectionInfo, subscription_id: uuid::Uuid, topic_hash: &[u8; 32], event_seq: u32, payload: &[u8]) {
        self.publishes.lock().push(CapturedPublish {
            info: info.clone(), subscription_id, topic_hash: *topic_hash, event_seq, payload: payload.to_vec(),
        });
    }

    fn on_reply(&self, info: &ConnectionInfo, message_id: uuid::Uuid, correlation_id: uuid::Uuid, status_phase: u32, payload: &[u8]) {
        self.replies.lock().push(CapturedReply {
            info: info.clone(), message_id, correlation_id, status_phase, payload: payload.to_vec(),
        });
    }

    fn on_reject(&self, info: &ConnectionInfo, rejected_message_id: uuid::Uuid, reason_code: u32, detail: &str) {
        self.rejects.lock().push(CapturedReject {
            info: info.clone(), rejected_message_id, reason_code, detail: detail.to_owned(),
        });
    }

    fn on_bulk_complete(&self, info: &ConnectionInfo, stream_id: u8, transfer_id: uuid::Uuid, total_bytes: u64, total_chunks: u32) {
        self.bulk_completes.lock().push(CapturedBulkComplete {
            info: info.clone(), stream_id, transfer_id, total_bytes, total_chunks,
        });
    }

    fn on_bulk_failed(&self, info: &ConnectionInfo, stream_id: u8, transfer_id: uuid::Uuid, reason: &str) {
        self.bulk_failures.lock().push(CapturedBulkFailed {
            info: info.clone(), stream_id, transfer_id, reason: reason.to_owned(),
        });
    }

    fn on_bulk_chunk(&self, info: &ConnectionInfo, stream_id: u8, chunk_index: u32, data: &[u8]) {
        self.bulk_chunks.lock().push(CapturedBulkChunk {
            info: info.clone(), stream_id, chunk_index, data: data.to_vec(),
        });
    }

    fn on_ack(&self, info: &ConnectionInfo, message_ids: &[uuid::Uuid]) {
        self.acks.lock().push(CapturedAck {
            info: info.clone(), message_ids: message_ids.to_vec(),
        });
    }

    fn on_connection_state_change(&self, info: &ConnectionInfo, old_phase: ConnectionPhase, new_phase: ConnectionPhase) {
        self.state_changes.lock().push(CapturedStateChange {
            info: info.clone(), old_phase, new_phase,
        });
    }

    fn on_arena_write(&self, info: &ConnectionInfo, shmref: &crate::v4::streaming::shared_arena::SharedMemRef, data: &[u8]) {
        self.arena_writes.lock().push(CapturedArenaWrite {
            info: info.clone(), shmref: *shmref, data: data.to_vec(),
        });
    }

    fn on_dmabuf_ref(&self, info: &ConnectionInfo, dmabuf: &crate::v4::streaming::dmabuf::DmaBufRef, payload_id: u8) {
        self.dmabuf_refs.lock().push(CapturedDmaBufRef {
            info: info.clone(), dmabuf: *dmabuf, payload_id,
        });
    }
}

// ── ReplyRouter — production-quality FrameRouter that replies ────

/// A FrameRouter that delegates to an inner MockRouter for capture AND
/// sends DATAGRAM_REPLY for every on_request. This is the minimum
/// viable FrameRouter for any bidirectional transport consumer.
///
/// rekindle-node's DaemonRouter does this plus application logic.
/// Tests and benches use ReplyRouter directly. The transport requires
/// replies to flow back through the ConnectionHandle's outbound_tx —
/// without this, send_request callers never receive their ACK.
pub struct ReplyRouter {
    pub router: Arc<MockRouter>,
    pub conn_handle: crate::v4::server::ConnectionHandle,
}

impl FrameRouter for ReplyRouter {
    fn route_frame(&self, info: &ConnectionInfo, class: u8, kind: u8, payload: &[u8]) {
        self.router.route_frame(info, class, kind, payload);
    }

    fn on_request(&self, info: &ConnectionInfo, message_id: uuid::Uuid, sender_clearance: Clearance, payload: &[u8]) {
        self.router.on_request(info, message_id, sender_clearance, payload);
        let reply_payload = crate::v4::codec::datagram::reply::encode(
            &crate::v4::codec::datagram::reply::DatagramReplyPayload {
                message_id: uuid::Uuid::now_v7(),
                correlation_id: message_id,
                status_phase: 0x03,
                application_payload: payload.to_vec(),
                status_reason: String::new(),
                status_message: String::new(),
                conditions: vec![],
            },
        );
        let frame = crate::v4::wire::outbound::OutboundFrame::Datagram {
            kind: crate::v4::wire::frame_kind::DatagramKind::Reply,
            payload: reply_payload,
        };
        if self.conn_handle.outbound_tx.send(frame).is_err() {
            tracing::error!(
                conn_id = info.conn_id,
                %message_id,
                "ReplyRouter: outbound_tx closed — DATAGRAM_REPLY dropped, connection dead"
            );
        }
    }

    fn on_notify(&self, info: &ConnectionInfo, message_id: uuid::Uuid, sender_clearance: Clearance, payload: &[u8]) {
        self.router.on_notify(info, message_id, sender_clearance, payload);
    }

    fn on_publish(&self, info: &ConnectionInfo, subscription_id: uuid::Uuid, topic_hash: &[u8; 32], event_seq: u32, payload: &[u8]) {
        self.router.on_publish(info, subscription_id, topic_hash, event_seq, payload);
    }

    fn on_reply(&self, info: &ConnectionInfo, message_id: uuid::Uuid, correlation_id: uuid::Uuid, status_phase: u32, payload: &[u8]) {
        self.router.on_reply(info, message_id, correlation_id, status_phase, payload);
    }

    fn on_reject(&self, info: &ConnectionInfo, rejected_message_id: uuid::Uuid, reason_code: u32, detail: &str) {
        self.router.on_reject(info, rejected_message_id, reason_code, detail);
    }

    fn on_bulk_complete(&self, info: &ConnectionInfo, stream_id: u8, transfer_id: uuid::Uuid, total_bytes: u64, total_chunks: u32) {
        self.router.on_bulk_complete(info, stream_id, transfer_id, total_bytes, total_chunks);
    }

    fn on_bulk_failed(&self, info: &ConnectionInfo, stream_id: u8, transfer_id: uuid::Uuid, reason: &str) {
        self.router.on_bulk_failed(info, stream_id, transfer_id, reason);
    }

    fn on_bulk_chunk(&self, info: &ConnectionInfo, stream_id: u8, chunk_index: u32, data: &[u8]) {
        self.router.on_bulk_chunk(info, stream_id, chunk_index, data);
    }

    fn on_ack(&self, info: &ConnectionInfo, message_ids: &[uuid::Uuid]) {
        self.router.on_ack(info, message_ids);
    }

    fn on_connection_state_change(&self, info: &ConnectionInfo, old_phase: ConnectionPhase, new_phase: ConnectionPhase) {
        self.router.on_connection_state_change(info, old_phase, new_phase);
    }

    fn on_arena_write(&self, info: &ConnectionInfo, shmref: &crate::v4::streaming::shared_arena::SharedMemRef, data: &[u8]) {
        self.router.on_arena_write(info, shmref, data);
    }

    fn on_dmabuf_ref(&self, info: &ConnectionInfo, dmabuf: &crate::v4::streaming::dmabuf::DmaBufRef, payload_id: u8) {
        self.router.on_dmabuf_ref(info, dmabuf, payload_id);
    }
}

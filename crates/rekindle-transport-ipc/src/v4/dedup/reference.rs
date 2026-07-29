//! STREAM_REFERENCE decision logic — sender and receiver sides.

use super::cache::{SenderCache, ReceiverCache};
use super::clearance_binding::check_clearance;
use crate::v4::wire::clearance::Clearance;

// ── Sender decision ───────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
pub enum SenderDecision {
    SendFull,
    SendReference,
}

/// Decide whether to send a STREAM_REFERENCE or the full payload.
pub fn sender_decision(
    cache: &SenderCache,
    content_hash: &[u8; 32],
    session_id: uuid::Uuid,
    peer_id: [u8; 32],
) -> SenderDecision {
    if cache.is_acked(content_hash, session_id, peer_id) {
        SenderDecision::SendReference
    } else {
        SenderDecision::SendFull
    }
}

// ── Receiver decision ─────────────────────────────────────────────

#[derive(Debug)]
pub enum ReceiverDecision {
    CacheHit { payload: Vec<u8> },
    CacheMiss,
    CacheMismatch,
    ClearanceDenied,
}

/// Decide whether a STREAM_REFERENCE can be served from the receiver cache.
pub fn receiver_decision(
    cache: &ReceiverCache,
    content_hash: &[u8; 32],
    expected_bytes: u64,
    expected_chunks: u32,
    sender_clearance: Clearance,
) -> ReceiverDecision {
    let Some(entry) = cache.lookup(content_hash) else {
        return ReceiverDecision::CacheMiss;
    };

    if entry.payload_size_bytes != expected_bytes || entry.chunk_count != expected_chunks {
        return ReceiverDecision::CacheMismatch;
    }

    if !check_clearance(entry.clearance, sender_clearance) {
        return ReceiverDecision::ClearanceDenied;
    }

    ReceiverDecision::CacheHit {
        payload: entry.payload.clone(),
    }
}

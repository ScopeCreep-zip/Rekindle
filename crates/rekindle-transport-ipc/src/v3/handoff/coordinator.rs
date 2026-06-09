//! HandoffCoordinator — platform-independent handoff lifecycle state machine.

use std::collections::HashMap;

use crate::v3::handoff::credit::{SideChannelCreditTracker, CreditError};
use crate::v3::handoff::transport::{FdTransport, MemfdOps, FdTag, TransportError, MemfdError};

#[derive(Debug)]
pub enum HandoffError {
    CreditExhausted,
    TransportFailed(TransportError),
    MemfdFailed(MemfdError),
    MacFailed,
    VerifyFailed { expected: [u8; 32], computed: [u8; 32] },
    ContentHashMismatch { claimed: [u8; 32], actual: [u8; 32] },
    NotFound(uuid::Uuid),
    AlreadyCompleted(uuid::Uuid),
}

#[derive(Debug)]
pub enum HandoffOutcome {
    Delivered {
        duration_us: u64,
        stream_id: u8,
        content_hash: [u8; 32],
        payload_id: u32,
    },
    Rejected { reason: HandoffRejectReason, stream_id: u8 },
    TimedOut,
    FallbackRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffRejectReason {
    ContentHashMismatch,
    SizeMismatch,
    MacFailed,
    MmapFailed,
    FdTimeout,
}

#[derive(Debug, Clone)]
pub struct HandoffOfferInfo {
    pub handoff_id: uuid::Uuid,
    pub stream_id: u8,
    pub content_hash: [u8; 32],
    pub handoff_mac: [u8; 32],
    pub payload_id: u32,
}

struct PendingHandoff {
    stream_id: u8,
    content_hash: [u8; 32],
    payload_id: u32,
    started_at: std::time::Instant,
}

pub struct HandoffCoordinator<T: FdTransport, M: MemfdOps> {
    handoff_key: [u8; 32],
    credit: SideChannelCreditTracker,
    transport: T,
    memfd_ops: M,
    pending: HashMap<uuid::Uuid, PendingHandoff>,
}

impl<T: FdTransport, M: MemfdOps> HandoffCoordinator<T, M> {
    pub fn new(
        handoff_key: [u8; 32],
        credit: SideChannelCreditTracker,
        transport: T,
        memfd_ops: M,
    ) -> Self {
        Self {
            handoff_key,
            credit,
            transport,
            memfd_ops,
            pending: HashMap::new(),
        }
    }

    pub fn begin_handoff(
        &mut self,
        stream_id: u8,
        payload: &[u8],
        content_hash: [u8; 32],
    ) -> Result<HandoffOfferInfo, HandoffError> {
        self.credit.try_consume()
            .map_err(|CreditError::Exhausted| HandoffError::CreditExhausted)?;

        let (payload_id, computed_hash) = self.memfd_ops.create_and_seal(payload)
            .map_err(HandoffError::MemfdFailed)?;

        // Verify the memfd content matches the caller's claimed hash
        if computed_hash != content_hash {
            return Err(HandoffError::ContentHashMismatch {
                claimed: content_hash,
                actual: computed_hash,
            });
        }

        let handoff_id = uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
        let handoff_mac = compute_handoff_mac(&self.handoff_key, &content_hash, &handoff_id, stream_id);

        let tag = FdTag {
            stream_id,
            flags: 0x01, // SEALED
            payload_size: payload.len() as u64,
            payload_id,
        };
        self.transport.send_fd(&tag)
            .map_err(HandoffError::TransportFailed)?;

        self.pending.insert(handoff_id, PendingHandoff {
            stream_id,
            content_hash,
            payload_id,
            started_at: std::time::Instant::now(),
        });

        Ok(HandoffOfferInfo {
            handoff_id,
            stream_id,
            content_hash,
            handoff_mac,
            payload_id,
        })
    }

    pub fn receive_offer(&mut self, offer: &HandoffOfferInfo) -> Result<HandoffAcceptInfo, HandoffError> {
        // Verify MAC before trusting any offer fields
        let expected_mac = compute_handoff_mac(
            &self.handoff_key, &offer.content_hash, &offer.handoff_id, offer.stream_id,
        );
        if expected_mac != offer.handoff_mac {
            return Err(HandoffError::MacFailed);
        }

        // Receive the memfd fd from the sidechannel. The sender called
        // transport.send_fd() which sent the fd via SCM_RIGHTS. This
        // recv_fd() receives it and stores it in the shared fd_store
        // under the sender's payload_id so memfd_ops.verify() can find it.
        self.transport.recv_fd()
            .map_err(HandoffError::TransportFailed)?;

        // Verify memfd content against claimed hash
        let payload = self.memfd_ops.verify(offer.payload_id, &offer.content_hash)
            .map_err(|e| match e {
                MemfdError::VerifyFailed { expected, computed } => {
                    HandoffError::VerifyFailed { expected, computed }
                }
                other => HandoffError::MemfdFailed(other),
            })?;

        Ok(HandoffAcceptInfo {
            handoff_id: offer.handoff_id,
            verified_content_hash: offer.content_hash,
            payload,
        })
    }

    pub fn receive_accept(&mut self, handoff_id: uuid::Uuid) -> Result<HandoffOutcome, HandoffError> {
        let pending = self.pending.remove(&handoff_id)
            .ok_or(HandoffError::NotFound(handoff_id))?;
        let duration_us = u64::try_from(pending.started_at.elapsed().as_micros()).unwrap_or(u64::MAX);

        Ok(HandoffOutcome::Delivered {
            duration_us,
            stream_id: pending.stream_id,
            content_hash: pending.content_hash,
            payload_id: pending.payload_id,
        })
    }

    pub fn receive_reject(
        &mut self,
        handoff_id: uuid::Uuid,
        reason: HandoffRejectReason,
    ) -> HandoffOutcome {
        let stream_id = self.pending.remove(&handoff_id)
            .map_or(0, |p| p.stream_id);
        HandoffOutcome::Rejected { reason, stream_id }
    }

    pub fn cancel(&mut self, handoff_id: uuid::Uuid) {
        self.pending.remove(&handoff_id);
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

#[derive(Debug)]
pub struct HandoffAcceptInfo {
    pub handoff_id: uuid::Uuid,
    pub verified_content_hash: [u8; 32],
    pub payload: Vec<u8>,
}

fn compute_handoff_mac(
    key: &[u8; 32],
    content_hash: &[u8; 32],
    handoff_id: &uuid::Uuid,
    stream_id: u8,
) -> [u8; 32] {
    let mut data = Vec::with_capacity(32 + 16 + 1);
    data.extend_from_slice(content_hash);
    data.extend_from_slice(handoff_id.as_bytes());
    data.push(stream_id);
    *blake3::keyed_hash(key, &data).as_bytes()
}

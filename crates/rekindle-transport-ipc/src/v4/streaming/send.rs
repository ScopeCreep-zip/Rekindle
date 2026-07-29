//! Outbound streaming send — tier-aware frame dispatch.
//!
//! `StreamingSender` wraps a SharedArena, sidechannel, and outbound channel.
//! Routes frames to the optimal transport tier:
//! - Inline: returns `UseInline` — caller uses BulkSender or datagram
//! - SharedMem: acquires slot, writes payload, publishes, sends SharedMemRef
//! - DmaBuf: sends dmabuf fd via sidechannel, sends DmaBufRef metadata via control channel

use std::os::unix::io::{AsRawFd, BorrowedFd};
use std::sync::Arc;

use crate::v4::wire::outbound::OutboundFrame;
use crate::v4::wire::frame_kind::HandoffKind;

use super::dmabuf::DmaBufRef;
use super::shared_arena::{SharedArena, SharedMemRef};
use super::sidechannel::SideChannel;
use super::tier;
use super::TransferTier;

/// Module-level Result alias for streaming send operations.
pub type Result<T> = std::result::Result<T, StreamingSendError>;

/// Errors from streaming send.
#[derive(Debug)]
pub enum StreamingSendError {
    /// All arena slots busy — caller should drop frame or backpressure.
    AllSlotsBusy,
    /// Outbound channel closed — connection dead.
    ChannelClosed,
    /// Payload exceeds arena slot size.
    PayloadTooLarge { payload_len: usize, slot_size: usize },
    /// Sidechannel fd send failed.
    SideChannelFailed(super::sidechannel::SideChannelError),
    /// No sidechannel available — SHARED_ARENA not negotiated or sidechannel not established.
    NoSideChannel,
}

impl std::fmt::Display for StreamingSendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AllSlotsBusy => write!(f, "all arena slots busy"),
            Self::ChannelClosed => write!(f, "outbound channel closed"),
            Self::PayloadTooLarge { payload_len, slot_size } => {
                write!(f, "payload {payload_len} exceeds slot size {slot_size}")
            }
            Self::SideChannelFailed(e) => write!(f, "sidechannel fd send failed: {e}"),
            Self::NoSideChannel => write!(f, "no sidechannel available — SHARED_ARENA not negotiated"),
        }
    }
}

impl std::error::Error for StreamingSendError {}

/// Result of a streaming send.
#[must_use]
#[derive(Debug)]
pub enum StreamingSendResult {
    /// Payload ≤ 64 KiB or no arena — caller should use BulkSender or inline datagram.
    UseInline,
    /// Payload written to arena slot and SharedMemRef sent over the control channel.
    SentViaArena(SharedMemRef),
    /// DMA-BUF fd sent via sidechannel and DmaBufRef metadata sent over the control channel.
    SentViaDmaBuf,
}

/// Tier-aware outbound streaming sender.
///
/// Owned per-connection. The capture daemon calls `send_frame` for CPU-accessible
/// payloads and `send_dmabuf` for GPU buffer references. The sender selects the
/// optimal tier and dispatches.
///
/// `send_frame` takes `&self` — all mutation goes through SharedArena's
/// AtomicU64 CAS (interior mutability via atomics, lock-free).
#[derive(Clone)]
pub struct StreamingSender {
    arena: Option<Arc<SharedArena>>,
    arena_id: u8,
    /// SEQPACKET sidechannel for DMA-BUF fd passing.
    /// Shared with the arena setup path. None if no sidechannel was negotiated.
    sidechannel: Option<Arc<SideChannel>>,
    outbound_tx: tokio::sync::mpsc::UnboundedSender<OutboundFrame>,
}

impl StreamingSender {
    pub fn new(
        arena: Option<Arc<SharedArena>>,
        arena_id: u8,
        sidechannel: Option<Arc<SideChannel>>,
        outbound_tx: tokio::sync::mpsc::UnboundedSender<OutboundFrame>,
    ) -> Self {
        Self { arena, arena_id, sidechannel, outbound_tx }
    }

    /// Whether the arena is available for this connection.
    pub fn has_arena(&self) -> bool {
        self.arena.is_some()
    }

    /// Whether the sidechannel is available for DMA-BUF fd passing.
    pub fn has_sidechannel(&self) -> bool {
        self.sidechannel.is_some()
    }

    /// Send a CPU-accessible payload via the optimal tier.
    ///
    /// For GPU buffers, use `send_dmabuf` instead — it sends the fd via
    /// sidechannel without touching the payload bytes.
    ///
    /// Returns `UseInline` if the payload should be sent via BulkSender
    /// or inline datagram. Returns `SentViaArena` if the payload was
    /// written to an arena slot and SharedMemRef was sent.
    pub async fn send_frame(
        &self,
        payload: &[u8],
    ) -> Result<StreamingSendResult> {
        let tier = match &self.arena {
            Some(arena) => tier::select_tier(payload.len(), Some(arena.as_ref()), false),
            None => TransferTier::Inline,
        };

        match tier {
            TransferTier::Inline => Ok(StreamingSendResult::UseInline),

            TransferTier::SharedMem => {
                let arena = self.arena.as_ref()
                    .expect("select_tier returned SharedMem without arena");

                if payload.len() > arena.slot_size() {
                    return Err(StreamingSendError::PayloadTooLarge {
                        payload_len: payload.len(),
                        slot_size: arena.slot_size(),
                    });
                }

                let mut guard = arena.try_acquire()
                    .ok_or(StreamingSendError::AllSlotsBusy)?;

                guard.write_from_slice(payload);
                let shmref = guard.release(self.arena_id, payload.len());

                // Read integrity from the arena itself — never from StreamingSender's
                // field, which could diverge if constructed inconsistently.
                let integrity = arena.integrity_check();
                let wire_size = crate::v4::codec::streaming::arena_write::wire_size(integrity);
                let mut buf = [0u8; 47]; // max wire size (with digest)
                crate::v4::codec::streaming::arena_write::encode(&shmref, &mut buf[..wire_size], integrity);

                self.outbound_tx.send(OutboundFrame::Handoff {
                    kind: HandoffKind::ArenaWrite,
                    payload: buf[..wire_size].to_vec(),
                }).map_err(|_| StreamingSendError::ChannelClosed)?;

                Ok(StreamingSendResult::SentViaArena(shmref))
            }

            // select_tier with has_dmabuf_fd=false never returns DmaBuf
            TransferTier::DmaBuf => unreachable!("select_tier(has_dmabuf_fd=false) cannot return DmaBuf"),
        }
    }

    /// Send a DMA-BUF reference for GPU-to-GPU transfer (Tier 3).
    ///
    /// The dmabuf fd is sent via the SEQPACKET sidechannel (SCM_RIGHTS).
    /// The DmaBufRef metadata is sent via the Handoff lane (encrypted control channel).
    /// The receiver imports the fd into their GPU context — zero CPU copy.
    ///
    /// `dmabuf_fd`: the DMA-BUF fd exported by the GPU driver.
    /// `fence_fd`: optional sync_file fd for explicit GPU fence synchronization.
    ///   Pass None for implicit sync (DRM driver handles it).
    /// `metadata`: frame dimensions, pixel format, plane layout.
    pub async fn send_dmabuf(
        &self,
        dmabuf_fd: BorrowedFd<'_>,
        fence_fd: Option<BorrowedFd<'_>>,
        metadata: &DmaBufRef,
    ) -> Result<StreamingSendResult> {
        let sc = self.sidechannel.as_ref()
            .ok_or(StreamingSendError::NoSideChannel)?;

        // Send dmabuf fd (and optional fence fd) via sidechannel SCM_RIGHTS.
        // Tagged with payload_id_hint for correlation with the DmaBufRef metadata.
        let tag = super::sidechannel::FdTag {
            stream_id: self.arena_id,
            flags: if fence_fd.is_some() { 0x02 } else { 0x01 }, // 0x01 = dmabuf only, 0x02 = dmabuf + fence
            payload_size: 0,
            payload_id: metadata.payload_id_hint as u32,
            fd: dmabuf_fd.as_raw_fd(),
        };
        sc.send_tagged_fd(&tag)
            .map_err(StreamingSendError::SideChannelFailed)?;

        // If fence fd present, send it as a second tagged fd
        if let Some(fence) = fence_fd {
            let fence_tag = super::sidechannel::FdTag {
                stream_id: self.arena_id,
                flags: 0x03, // 0x03 = fence fd
                payload_size: 0,
                payload_id: metadata.payload_id_hint as u32,
                fd: fence.as_raw_fd(),
            };
            sc.send_tagged_fd(&fence_tag)
                .map_err(StreamingSendError::SideChannelFailed)?;
        }

        // Send DmaBufRef metadata via Handoff lane (encrypted control channel)
        let mut buf = [0u8; DmaBufRef::WIRE_SIZE];
        crate::v4::codec::streaming::dmabuf_ref::encode(metadata, &mut buf);

        self.outbound_tx.send(OutboundFrame::Handoff {
            kind: HandoffKind::DmaBufRef,
            payload: buf.to_vec(),
        }).map_err(|_| StreamingSendError::ChannelClosed)?;

        Ok(StreamingSendResult::SentViaDmaBuf)
    }
}

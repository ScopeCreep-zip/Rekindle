//! Connection control loop — per-lane task architecture.
//!
//! Each connection spawns 4 lane tasks + 1 audit merge task:
//!
//! ```text
//! read task (io_uring)
//!   ├─► lane::control::run  — Channel + Datagram, heartbeat, lifecycle
//!   ├─► lane::data::run     — Stream, bulk decrypt, FIN tracking
//!   ├─► lane::audit::run    — Audit
//!   └─► lane::handoff::run  — Streaming (arena, DMA-BUF)
//!         │
//!         └─► audit_merge::run — collects LinkInputs, maintains chains
//! ```
//!
//! Invariant: no lane task calls any blocking function. Every channel
//! send is tokio mpsc `.send().await`. Crossbeam channels are used
//! ONLY by rayon workers (OS threads) and the write task.

pub(crate) mod audit_reorder;
pub(crate) mod audit_merge;
pub(crate) mod fin;
pub(crate) mod heartbeat;
pub(crate) mod util;
pub(crate) mod shared_state;
pub(crate) mod lane;

use crate::v4::io::lane_channels::PlaintextBuf;
use crate::v4::wire::constants::ENVELOPE_LEN;

use super::read_task::SessionOutcome;

// ── Public types — contract between read task and lane tasks ────

pub struct VerifiedFrame {
    pub envelope_bytes: [u8; ENVELOPE_LEN],
    pub envelope: crate::v4::codec::envelope::EnvelopeInfo,
    pub header: Option<crate::v4::codec::header::StreamHeaderInfo>,
    pub plaintext: Vec<u8>,
    pub peer_epoch_advanced: bool,
    pub audit_envelope_hash: [u8; 32],
    pub audit_header_hash: [u8; 32],
    pub audit_ciphertext_hash: [u8; 32],
    pub retained_wire: Vec<u8>,
}

pub enum ReadSignal {
    Frame(VerifiedFrame),
    Finished(SessionOutcome),
}

/// Signal from lane tasks to the bridge task for bulk data delivery.
pub enum BulkDataSignal {
    Chunk { stream_id: u8, chunk_index: u32, data: PlaintextBuf },
    Complete { stream_id: u8, transfer_id: uuid::Uuid, total_bytes: u64, total_chunks: u32 },
}

/// Write task error.
#[derive(Debug)]
pub enum WriteError {
    Io(std::io::Error),
}

impl From<std::io::Error> for WriteError {
    fn from(e: std::io::Error) -> Self { Self::Io(e) }
}

// ── Per-lane inbound channels for read task demux ──────────────

pub const CONTROL_CHANNEL_CAPACITY: usize = 64;
pub const DATA_CHANNEL_CAPACITY: usize = 256;
pub const AUDIT_CHANNEL_CAPACITY: usize = 64;
pub const HANDOFF_CHANNEL_CAPACITY: usize = 1024;

pub struct LaneInboundChannels {
    pub control_tx: tokio::sync::mpsc::Sender<ReadSignal>,
    pub data_tx: tokio::sync::mpsc::Sender<ReadSignal>,
    pub audit_tx: tokio::sync::mpsc::Sender<ReadSignal>,
    pub handoff_tx: tokio::sync::mpsc::Sender<ReadSignal>,
}

pub struct LaneInboundReceivers {
    pub control_rx: tokio::sync::mpsc::Receiver<ReadSignal>,
    pub data_rx: tokio::sync::mpsc::Receiver<ReadSignal>,
    pub audit_rx: tokio::sync::mpsc::Receiver<ReadSignal>,
    pub handoff_rx: tokio::sync::mpsc::Receiver<ReadSignal>,
}

pub fn create_lane_inbound_channels() -> (LaneInboundChannels, LaneInboundReceivers) {
    let (control_tx, control_rx) = tokio::sync::mpsc::channel(CONTROL_CHANNEL_CAPACITY);
    let (data_tx, data_rx) = tokio::sync::mpsc::channel(DATA_CHANNEL_CAPACITY);
    let (audit_tx, audit_rx) = tokio::sync::mpsc::channel(AUDIT_CHANNEL_CAPACITY);
    let (handoff_tx, handoff_rx) = tokio::sync::mpsc::channel(HANDOFF_CHANNEL_CAPACITY);

    (
        LaneInboundChannels { control_tx, data_tx, audit_tx, handoff_tx },
        LaneInboundReceivers { control_rx, data_rx, audit_rx, handoff_rx },
    )
}

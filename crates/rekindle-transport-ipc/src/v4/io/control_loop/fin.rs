//! Bulk transfer FIN tracking types.
//!
//! These types are the interface between BulkSender (which produces
//! PendingFin messages) and the Data lane task (which tracks
//! PendingFinState for FIN emission and PendingFinVerify for
//! receive-side content hash verification).
//!
//! The FIN lifecycle logic (emit_stream_fin, check_deferred_fin_verify)
//! lives in lane/data.rs where it has direct access to DataState fields.

use crate::v4::io::lane_channels::BulkFrame;

/// Sent by BulkSender to the Data lane after spawning all rayon workers.
/// The Data lane counts arriving OutboundAuditLinks and emits STREAM_FIN
/// through bulk_wire_tx when all payload LinkInputs have been processed.
pub struct PendingFin {
    pub stream_id: u8,
    pub chunk_count: u32,
    pub total_bytes: u64,
    pub content_hash: [u8; 32],
    pub bulk_wire_tx: crossbeam::channel::Sender<BulkFrame>,
    /// The session_seq of the last payload chunk. The outbound audit
    /// reorder buffer must advance past this seq before FIN can be emitted.
    pub last_chunk_seq: u64,
}

/// Data lane state for a pending FIN emission.
pub struct PendingFinState {
    pub chunk_count: u32,
    pub total_bytes: u64,
    pub content_hash: [u8; 32],
    pub bulk_wire_tx: crossbeam::channel::Sender<BulkFrame>,
    /// The session_seq of the last payload chunk. FIN is emitted when
    /// outbound_reorder.next_expected() > last_chunk_seq — meaning all
    /// payload LinkInputs have been flushed through the audit chain.
    pub last_chunk_seq: u64,
}

/// Receive-side: stored when a FIN chunk arrives before all payload chunks.
/// Verification is deferred until the reassembler has all chunks.
pub struct PendingFinVerify {
    pub content_hash: [u8; 32],
    pub audit_link: [u8; 32],
    pub total_bytes: u64,
    pub transfer_id: uuid::Uuid,
    /// Total payload chunk count (indices 0..expected_chunks-1).
    /// Equals the FIN frame's chunk_index field. The reassembler is
    /// complete when next_expected >= expected_chunks && buffered == 0.
    pub expected_chunks: u32,
}

//! Audit merge task — single ordered sink for all 4 lane tasks.
//!
//! All lanes send `AuditLinkDirection` via `.send().await` (NEVER try_send).
//! The merge task reorders by session_seq and advances the correct chain.
//! A dropped LinkInput deadlocks the reorder buffer permanently.
//!
//! Owns both audit chains, both reorder buffers, both checkpoint cadences,
//! stall-based gap detection, and the checkpoint emission encoder. No other
//! task touches audit chains.
//!
//! Every frame audit_merge encodes (checkpoints, gap frames) gets its audit
//! link fed back into the outbound chain via encode_send_and_chain. The
//! outbound reorder buffer sees every session_seq. No gaps. FIN readiness
//! advances. Bulk transfers complete.
//!
//! Gap detection is stall-based: when the inbound reorder buffer has entries
//! buffered AND time_since_last_advance exceeds max_stall_duration, the
//! buffer is stalled on a genuinely missing seq. Rayon completion reordering
//! resolves in milliseconds — a stall lasting hundreds of milliseconds means
//! the frame is lost, not temporarily late. The gap_range() method on the
//! reorder buffer identifies the exact missing seqs. AUDIT_GAP is emitted
//! once per stall detection, not on every out-of-order arrival.
//!
//! Publishes chain metadata to SharedAuditLinks on every advance:
//! current_link, anchor, length, outbound_next_seq, recv_last_seq
//! via AtomicU64 fields. Zero allocation on the hot path. No per-frame cloning.
//!
//! recv_last_seq is the SSOT for global inbound progress across ALL lanes.
//! Heartbeat reads it for PING last_seen_remote_seq. No per-lane shadow.
//!
//! Stall detection: if a reorder buffer has entries buffered and hasn't
//! advanced for longer than max_stall_duration, the merge task emits
//! AUDIT_GAP for the inbound direction. For the outbound direction, stall
//! means a rayon worker panicked — session terminates.

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::v4::audit::chain::{AuditChain, LinkInput};
use crate::v4::audit::checkpoint::CheckpointTracker;
use crate::v4::audit::gap::MissingBitmapBuilder;
use crate::v4::codec::audit::checkpoint as checkpoint_codec;
use crate::v4::codec::audit::gap as gap_codec;
use crate::v4::io::encode::{FrameEncoder, SmallFrameEncoder};
use crate::v4::io::lane_channels::LaneChannels;
use crate::v4::wire::frame_kind::AuditKind;
use crate::v4::wire::lane::Lane;
use crate::v4::wire::outbound::OutboundFrame;

use super::audit_reorder::AuditReorderBuffer;
use super::shared_state::SharedAuditLinks;
use super::util;

/// Messages sent to the audit merge task from lane tasks.
pub enum AuditLinkDirection {
    Inbound(LinkInput),
    Outbound(LinkInput),
    GraftInbound {
        link_inputs: Vec<LinkInput>,
    },
    RotationAnchor {
        outbound_key: [u8; 32],
        inbound_key: [u8; 32],
        rotation_link: [u8; 32],
        start_session_seq: u64,
    },
}

/// Encode an outbound audit/gap/checkpoint frame, send wire bytes, and feed
/// the audit link back into the outbound chain.
async fn encode_send_and_chain(
    frame: &OutboundFrame,
    small_encoder: &mut SmallFrameEncoder,
    encoder: &Arc<FrameEncoder>,
    lane_channels: &LaneChannels,
    outbound_reorder: &mut AuditReorderBuffer,
    outbound_chain: &mut AuditChain,
    outbound_cp: &mut CheckpointTracker,
    audit_links: &Arc<SharedAuditLinks>,
    label: &str,
) {
    let encoded = small_encoder.encode_small(
        encoder, Lane::Audit,
        frame.class_byte(), frame.kind_byte(), frame.payload_bytes(),
    );
    let link = encoded.link_input;
    let wire = encoded.wire.to_vec();
    tracing::debug!(
        session_seq = link.session_seq,
        label,
        wire_len = wire.len(),
        "audit_merge: encode_send_and_chain — sending frame"
    );
    if lane_channels.send_with_retry(Lane::Audit, wire).await.is_err() {
        tracing::error!(label, session_seq = link.session_seq, "audit_merge: frame send failed");
    }
    outbound_reorder.insert_and_drain(link, |entry| {
        outbound_chain.advance(entry);
        audit_links.outbound_link.store(outbound_chain.current_link());
        audit_links.outbound_length.store(
            outbound_chain.length(),
            std::sync::atomic::Ordering::Release,
        );
        outbound_cp.frame_processed();
    });
    audit_links.notify_outbound_advance(outbound_reorder.next_expected());
}

pub async fn run(
    mut rx: mpsc::Receiver<AuditLinkDirection>,
    mut outbound_chain: AuditChain,
    mut inbound_chain: AuditChain,
    mut outbound_reorder: AuditReorderBuffer,
    mut inbound_reorder: AuditReorderBuffer,
    outbound_checkpoint: CheckpointTracker,
    inbound_checkpoint: CheckpointTracker,
    lane_channels: LaneChannels,
    encoder: Arc<FrameEncoder>,
    audit_links: Arc<SharedAuditLinks>,
) {
    let mut small_encoder = SmallFrameEncoder::new();
    let mut outbound_cp = outbound_checkpoint;
    let mut inbound_cp = inbound_checkpoint;
    let mut outbound_checkpoint_seq: u64 = 0;
    let mut inbound_checkpoint_seq: u64 = 0;

    tracing::debug!("audit_merge: task started");

    while let Some(directed) = rx.recv().await {
        match directed {
            AuditLinkDirection::Inbound(link) => {
                tracing::debug!(
                    session_seq = link.session_seq,
                    inbound_next = inbound_reorder.next_expected(),
                    inbound_buffered = inbound_reorder.buffered_count(),
                    "audit_merge: received Inbound link"
                );

                inbound_reorder.insert_and_drain(link, |entry| {
                    inbound_chain.advance(entry);
                    audit_links.inbound_link.store(inbound_chain.current_link());
                    audit_links.inbound_length.store(
                        inbound_chain.length(),
                        std::sync::atomic::Ordering::Release,
                    );
                    inbound_cp.frame_processed();
                });

                let recv_last = inbound_reorder.next_expected().saturating_sub(1);
                audit_links.recv_last_seq.store(
                    recv_last,
                    std::sync::atomic::Ordering::Release,
                );
                tracing::debug!(
                    recv_last_seq = recv_last,
                    inbound_chain_length = inbound_chain.length(),
                    inbound_next = inbound_reorder.next_expected(),
                    inbound_buffered = inbound_reorder.buffered_count(),
                    "audit_merge: inbound drain complete"
                );

                // Inbound checkpoint emission.
                if inbound_cp.is_due() {
                    let seq = inbound_checkpoint_seq;
                    inbound_checkpoint_seq += 1;
                    tracing::debug!(
                        checkpoint_seq = seq,
                        chain_length = inbound_chain.length(),
                        "audit_merge: emitting inbound checkpoint"
                    );
                    let payload = checkpoint_codec::encode(
                        &checkpoint_codec::AuditCheckpointPayload {
                            chain_index: inbound_chain.length(),
                            chain_length: inbound_chain.length(),
                            checkpoint_seq: seq,
                            wall_clock_ns: util::wall_ns(),
                            chain_link: inbound_chain.current_link(),
                            anchor_link: inbound_chain.anchor_record().value,
                        },
                    );
                    let frame = OutboundFrame::Audit {
                        kind: AuditKind::Checkpoint,
                        payload,
                    };
                    encode_send_and_chain(
                        &frame, &mut small_encoder, &encoder, &lane_channels,
                        &mut outbound_reorder, &mut outbound_chain, &mut outbound_cp,
                        &audit_links, "inbound_checkpoint",
                    ).await;
                    inbound_cp.checkpoint_emitted();
                }

                // Stall-based gap detection: if the inbound reorder buffer
                // has entries buffered and hasn't advanced for longer than
                // max_stall_duration, a frame is genuinely lost. Emit AUDIT_GAP
                // with the exact missing range so the peer can replay.
                if inbound_reorder.is_stalled() {
                    if let Some((gap_start, gap_end)) = inbound_reorder.gap_range() {
                        let missing_count = gap_end - gap_start + 1;
                        tracing::info!(
                            gap_start, gap_end, missing_count,
                            stall_ms = inbound_reorder.time_since_last_advance().as_millis() as u64,
                            buffered = inbound_reorder.buffered_count(),
                            "audit_merge: inbound stall detected — emitting AUDIT_GAP"
                        );
                        let mut builder = MissingBitmapBuilder::new(gap_start, gap_end);
                        for s in gap_start..=gap_end {
                            builder.mark_missing(s);
                        }
                        let bitmap = builder.build();
                        let gap_id = uuid::Uuid::now_v7();
                        let gap_payload = gap_codec::encode(
                            &gap_codec::AuditGapPayload {
                                gap_id,
                                gap_start_seq: gap_start,
                                gap_end_seq: gap_end,
                                gap_detected_ns: util::wall_ns(),
                                expected_chain_link: inbound_chain.current_link(),
                                missing_bitmap: bitmap.as_bytes().to_vec(),
                            },
                        );
                        let frame = OutboundFrame::Audit {
                            kind: AuditKind::Gap,
                            payload: gap_payload,
                        };
                        encode_send_and_chain(
                            &frame, &mut small_encoder, &encoder, &lane_channels,
                            &mut outbound_reorder, &mut outbound_chain, &mut outbound_cp,
                            &audit_links, "audit_gap",
                        ).await;
                    } else {
                        // Stalled but no gap range — stored_count > 0 but
                        // next_expected is filled. Should not happen. Session-fatal.
                        tracing::error!(
                            next = inbound_reorder.next_expected(),
                            buffered = inbound_reorder.buffered_count(),
                            stall_ms = inbound_reorder.time_since_last_advance().as_millis() as u64,
                            "audit_merge: inbound stall with no gap range — session terminating"
                        );
                        break;
                    }
                }
            }

            AuditLinkDirection::Outbound(link) => {
                tracing::debug!(
                    session_seq = link.session_seq,
                    outbound_next = outbound_reorder.next_expected(),
                    outbound_buffered = outbound_reorder.buffered_count(),
                    "audit_merge: received Outbound link"
                );

                outbound_reorder.insert_and_drain(link, |entry| {
                    outbound_chain.advance(entry);
                    audit_links.outbound_link.store(outbound_chain.current_link());
                    audit_links.outbound_length.store(
                        outbound_chain.length(),
                        std::sync::atomic::Ordering::Release,
                    );
                    outbound_cp.frame_processed();
                });

                let next_seq = outbound_reorder.next_expected();
                audit_links.notify_outbound_advance(next_seq);
                tracing::debug!(
                    outbound_next_seq = next_seq,
                    outbound_chain_length = outbound_chain.length(),
                    outbound_buffered = outbound_reorder.buffered_count(),
                    "audit_merge: outbound drain complete"
                );

                // Outbound checkpoint emission.
                if outbound_cp.is_due() {
                    let seq = outbound_checkpoint_seq;
                    outbound_checkpoint_seq += 1;
                    tracing::debug!(
                        checkpoint_seq = seq,
                        chain_length = outbound_chain.length(),
                        "audit_merge: emitting outbound checkpoint"
                    );
                    let payload = checkpoint_codec::encode(
                        &checkpoint_codec::AuditCheckpointPayload {
                            chain_index: outbound_chain.length(),
                            chain_length: outbound_chain.length(),
                            checkpoint_seq: seq,
                            wall_clock_ns: util::wall_ns(),
                            chain_link: outbound_chain.current_link(),
                            anchor_link: outbound_chain.anchor_record().value,
                        },
                    );
                    let frame = OutboundFrame::Audit {
                        kind: AuditKind::Checkpoint,
                        payload,
                    };
                    encode_send_and_chain(
                        &frame, &mut small_encoder, &encoder, &lane_channels,
                        &mut outbound_reorder, &mut outbound_chain, &mut outbound_cp,
                        &audit_links, "outbound_checkpoint",
                    ).await;
                    outbound_cp.checkpoint_emitted();
                }

                // Outbound stall = rayon worker panicked. No AUDIT_GAP to emit —
                // the outbound chain is local. Session must terminate.
                if outbound_reorder.is_stalled() {
                    tracing::error!(
                        next = outbound_reorder.next_expected(),
                        buffered = outbound_reorder.buffered_count(),
                        stall_ms = outbound_reorder.time_since_last_advance().as_millis() as u64,
                        "audit_merge: outbound reorder stalled — session terminating"
                    );
                    break;
                }
            }

            AuditLinkDirection::GraftInbound { link_inputs } => {
                tracing::debug!(
                    link_count = link_inputs.len(),
                    inbound_chain_length_before = inbound_chain.length(),
                    "audit_merge: GraftInbound received"
                );
                crate::v4::audit::replay::graft_frames(
                    &mut inbound_chain, &link_inputs, &[],
                );
                audit_links.inbound_link.store(inbound_chain.current_link());
                audit_links.inbound_length.store(
                    inbound_chain.length(),
                    std::sync::atomic::Ordering::Release,
                );
                tracing::debug!(
                    inbound_chain_length_after = inbound_chain.length(),
                    "audit_merge: GraftInbound applied"
                );
            }

            AuditLinkDirection::RotationAnchor {
                outbound_key, inbound_key, rotation_link, start_session_seq,
            } => {
                tracing::info!(
                    start_session_seq,
                    rotation_link_prefix = %hex::encode(&rotation_link[..8]),
                    outbound_chain_length = outbound_chain.length(),
                    inbound_chain_length = inbound_chain.length(),
                    "audit_merge: RotationAnchor received — resetting both chains"
                );
                outbound_chain = AuditChain::with_rotation_anchor(
                    outbound_key, rotation_link, start_session_seq,
                );
                inbound_chain = AuditChain::with_rotation_anchor(
                    inbound_key, rotation_link, start_session_seq,
                );
                audit_links.outbound_anchor.store(rotation_link);
                audit_links.inbound_anchor.store(rotation_link);
                audit_links.outbound_link.store(rotation_link);
                audit_links.inbound_link.store(rotation_link);
                audit_links.outbound_length.store(0, std::sync::atomic::Ordering::Release);
                audit_links.inbound_length.store(0, std::sync::atomic::Ordering::Release);
                tracing::debug!("audit_merge: RotationAnchor applied — chains reset");
            }
        }
    }
    tracing::debug!("audit_merge: task exiting");
}

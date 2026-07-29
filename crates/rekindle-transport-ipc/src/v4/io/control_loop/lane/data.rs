//! Data lane task — Stream (0x02) frames + bulk decrypt + FIN lifecycle.
//!
//! Five select! arms:
//! - ReadSignal::Frame — inline stream frames (open/close/fin/ack/payload)
//!   + rerouted credit/backpressure frames from Control lane
//! - BulkDecrypted — rayon-decrypted chunks → reassembler → delivery
//!   + inbound audit link forwarded to audit_merge
//! - OutboundAuditLink — bulk rayon outbound audit links → forwarded
//!   to audit_merge (single chain path, zero local reorder)
//! - PendingFinReceived — BulkSender signals when to emit STREAM_FIN
//! - DataRevocation — resume/dedup revocations forwarded from Control lane
//!
//! FIN readiness reads from SharedAuditLinks.outbound_next_seq — published
//! by audit_merge after every outbound chain drain. No local reorder buffer.

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::v4::audit::chain::LinkInput;
use crate::v4::bulk::recv::BulkDecryptResult;
use crate::v4::bulk::AuditQueue;
use crate::v4::handlers::HandlerError;
use crate::v4::io::control_loop::fin::{PendingFin, PendingFinState};
use crate::v4::io::control_loop::{BulkDataSignal, ReadSignal};
use crate::v4::router::{ConnectionInfo, FrameRouter};
use crate::v4::stream::registry::Direction;
use crate::v4::wire::frame_kind::StreamKind;
use crate::v4::wire::outbound::OutboundFrame;

use super::state::DataState;
use super::{extract_class_kind, FrameLoop, LaneShutdown};

use crate::v4::io::control_loop::shared_state::SharedAuditLinks;

/// Revocation forwarded from the Control lane's revoke handler.
pub enum DataRevocation {
    ResumeTransfer(uuid::Uuid),
    ContentHash([u8; 32]),
}

pub async fn run(
    mut rx: mpsc::Receiver<ReadSignal>,
    mut state: DataState,
    mut frame_loop: FrameLoop,
    router: Arc<dyn FrameRouter>,
    info: ConnectionInfo,
    bulk_decrypted_queue: Arc<rekindle_transport_buff::DispatchQueue<BulkDecryptResult, rekindle_transport_buff::adapters::tokio::TokioWake>>,
    outbound_audit_queue: AuditQueue,
    outbound_audit_wake: rekindle_transport_buff::adapters::tokio::TokioWake,
    mut pending_fin_rx: mpsc::Receiver<PendingFin>,
    mut revocation_rx: mpsc::Receiver<DataRevocation>,
    bulk_data_tx: mpsc::Sender<BulkDataSignal>,
    credit_guard: Arc<rekindle_transport_buff::CreditGuard>,
    audit_links: Arc<SharedAuditLinks>,
) {
    let bulk_decrypt_wake = bulk_decrypted_queue.wake_sink().clone();
    let mut outbound_seq_watch = audit_links.outbound_seq_watch_rx.clone();
    tracing::debug!("data lane: task started");

    loop {
        tokio::select! {
            biased;

            signal = rx.recv() => {
                match signal {
                    Some(ReadSignal::Frame(frame)) => {
                        tracing::debug!(
                            session_seq = frame.envelope.session_seq,
                            "data lane: inline frame received"
                        );
                        let link = match frame_loop.process_frame(&frame, |f, outbound| {
                            let (class, kind, payload) = extract_class_kind(f)
                                .ok_or(HandlerError::CodecFailed("frame too short".into()))?;
                            crate::v4::dispatch::inbound::dispatch_data(
                                &mut state, router.as_ref(), &info, outbound,
                                f.header.as_ref(), class, kind, payload,
                            )
                        }) {
                            Ok(link) => link,
                            Err(e) => {
                                tracing::error!(error = ?e, "data lane: handler error — constructing NACK");
                                if let Some(ref h) = frame.header {
                                    tracing::debug!(
                                        stream_id = h.stream_id,
                                        chunk_index = h.chunk_index,
                                        failure_code = ?e.failure_code(),
                                        "data lane: NACK payload for failed handler"
                                    );
                                    let nack_payload = crate::v4::codec::stream::nack::encode(
                                        &crate::v4::codec::stream::nack::StreamNackPayload {
                                            reason_code: e.failure_code(),
                                            rejected_chunk_idx: h.chunk_index,
                                            detail: format!("{e:?}"),
                                        },
                                    );
                                    state.pending_outbound.push(OutboundFrame::Data {
                                        stream_id: h.stream_id,
                                        kind: StreamKind::Nack,
                                        chunk_index: h.chunk_index,
                                        payload: nack_payload,
                                    });
                                }
                                let link = LinkInput {
                                    session_seq: frame.envelope.session_seq,
                                    envelope_hash: frame.audit_envelope_hash,
                                    header_hash: frame.audit_header_hash,
                                    ciphertext_hash: frame.audit_ciphertext_hash,
                                };
                                tracing::debug!(
                                    session_seq = frame.envelope.session_seq,
                                    "data lane: draining audit for errored frame"
                                );
                                if frame_loop.drain_and_emit_audit(link, frame.retained_wire).await.is_err() {
                                    tracing::debug!("data lane: audit_tx closed during error drain — exiting");
                                    break;
                                }
                                tracing::debug!("data lane: draining pending outbound after error");
                                if drain_pending_outbound(&mut state, &mut frame_loop).await.is_err() {
                                    tracing::debug!("data lane: pending outbound drain failed — exiting");
                                    break;
                                }
                                continue;
                            }
                        };

                        tracing::debug!(
                            session_seq = frame.envelope.session_seq,
                            pending_deliveries = state.pending_bulk_deliveries.len(),
                            "data lane: draining bulk deliveries"
                        );
                        if drain_bulk_deliveries(&mut state, &bulk_data_tx).await.is_err() {
                            tracing::debug!("data lane: bulk_data_tx closed — exiting");
                            break;
                        }

                        tracing::debug!(
                            session_seq = frame.envelope.session_seq,
                            "data lane: draining audit for inline frame"
                        );
                        if frame_loop.drain_and_emit_audit(link, frame.retained_wire).await.is_err() {
                            tracing::debug!("data lane: audit_tx closed during inline drain — exiting");
                            break;
                        }
                    }
                    Some(ReadSignal::Finished(outcome)) => {
                        tracing::info!(?outcome, "data lane: read task finished — shutting down");
                        break;
                    }
                    None => {
                        tracing::debug!("data lane: rx channel closed — exiting");
                        break;
                    }
                }
            }

            _ = bulk_decrypt_wake.notified() => {
                tracing::debug!("data lane: bulk_decrypt_wake fired");
                let mut count = 0u32;
                while let Some((_seq, result)) = bulk_decrypted_queue.pop() {
                    count += 1;
                    tracing::debug!(session_seq = _seq, batch_item = count, "data lane: processing bulk decrypt result");
                    if handle_bulk_decrypted(
                        &mut state, result, &frame_loop, &router, &info,
                        &bulk_data_tx, &credit_guard, &audit_links,
                    ).await.is_err() {
                        tracing::debug!(processed = count, "data lane: handle_bulk_decrypted fatal — exiting");
                        return;
                    }
                }
                tracing::debug!(processed = count, "data lane: bulk decrypt batch complete, draining pending outbound");
                if drain_pending_outbound(&mut state, &mut frame_loop).await.is_err() {
                    tracing::debug!("data lane: pending outbound drain failed after bulk — exiting");
                    return;
                }
            }

            _ = outbound_audit_wake.notified() => {
                tracing::debug!("data lane: outbound_audit_wake fired");
                let mut forward_count = 0u32;
                while let Some((_queue_seq, link)) = outbound_audit_queue.pop() {
                    forward_count += 1;
                    tracing::debug!(
                        session_seq = link.session_seq,
                        "data lane: forwarding outbound audit link to audit_merge"
                    );
                    crate::v4::bulk::counters::DIAG_AUDIT_OUTBOUND_POPS
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if frame_loop.audit_tx()
                        .send(crate::v4::io::control_loop::audit_merge::AuditLinkDirection::Outbound(link))
                        .await
                        .is_err()
                    {
                        tracing::error!(forwarded = forward_count, "data lane: audit_tx closed during outbound link forward");
                        return;
                    }
                }
                tracing::debug!(forwarded = forward_count, "data lane: outbound audit batch forwarded, checking FIN readiness");
                if emit_ready_fins(&mut state, &frame_loop, &audit_links).await.is_err() {
                    tracing::debug!("data lane: emit_ready_fins fatal — exiting");
                    return;
                }
            }

            Some(pf) = pending_fin_rx.recv() => {
                let sid = pf.stream_id;
                tracing::debug!(
                    stream_id = sid,
                    chunk_count = pf.chunk_count,
                    last_chunk_seq = pf.last_chunk_seq,
                    total_bytes = pf.total_bytes,
                    "data lane: PendingFin received"
                );
                state.pending_fins.insert(sid, PendingFinState {
                    chunk_count: pf.chunk_count,
                    total_bytes: pf.total_bytes,
                    content_hash: pf.content_hash,
                    bulk_wire_tx: pf.bulk_wire_tx,
                    last_chunk_seq: pf.last_chunk_seq,
                });
                tracing::debug!(
                    stream_id = sid,
                    pending_fin_count = state.pending_fins.len(),
                    "data lane: PendingFin stored, checking FIN readiness"
                );
                if emit_ready_fins(&mut state, &frame_loop, &audit_links).await.is_err() {
                    tracing::debug!("data lane: emit_ready_fins fatal after PendingFin — exiting");
                    return;
                }
            }

            Some(revocation) = revocation_rx.recv() => {
                match revocation {
                    DataRevocation::ResumeTransfer(transfer_id) => {
                        tracing::debug!(%transfer_id, "data lane: resume revocation applied");
                        state.resume_registry.remove(transfer_id);
                    }
                    DataRevocation::ContentHash(hash) => {
                        tracing::debug!(hash = %hex::encode(&hash[..8]), "data lane: content hash revocation applied");
                        state.sender_cache.remove(&hash);
                        state.receiver_cache.remove(&hash);
                    }
                }
            }

            Ok(()) = outbound_seq_watch.changed() => {
                let next_seq = *outbound_seq_watch.borrow_and_update();
                tracing::debug!(
                    next_seq,
                    pending_fin_count = state.pending_fins.len(),
                    "data lane: outbound_next_seq advanced via watch — checking FIN readiness"
                );
                if !state.pending_fins.is_empty() {
                    if emit_ready_fins(&mut state, &frame_loop, &audit_links).await.is_err() {
                        tracing::debug!("data lane: emit_ready_fins fatal after watch — exiting");
                        break;
                    }
                }
            }

            else => {
                tracing::debug!("data lane: all channels closed — exiting");
                break;
            }
        }
    }
    tracing::debug!("data lane: task exiting");
}

// ── Bulk decrypt handling ───────────────────────────────────────

async fn handle_bulk_decrypted(
    state: &mut DataState,
    result: BulkDecryptResult,
    frame_loop: &FrameLoop,
    router: &Arc<dyn FrameRouter>,
    info: &ConnectionInfo,
    bulk_data_tx: &mpsc::Sender<BulkDataSignal>,
    credit_guard: &rekindle_transport_buff::CreditGuard,
    audit_links: &SharedAuditLinks,
) -> Result<(), LaneShutdown> {
    match result {
        BulkDecryptResult::Chunk(chunk) => {
            let stream_id = chunk.stream_id;
            let chunk_index = chunk.chunk_index;
            let chunk_digest = chunk.chunk_digest;
            let kind = chunk.kind;
            let header_flags = chunk.header_flags;
            let inbound_link = chunk.link_input;

            tracing::debug!(
                stream_id, chunk_index, ?kind,
                has_reassembler = state.reassemblers.contains_key(&stream_id),
                header_flags,
                plaintext_len = chunk.plaintext.len(),
                session_seq = inbound_link.session_seq,
                "handle_bulk_decrypted: chunk arrived"
            );

            if kind == StreamKind::Fin {
                tracing::error!(
                    stream_id, chunk_index,
                    "BUG: FIN frame reached bulk decrypt path — bulk_threshold too low"
                );
                return Err(LaneShutdown);
            }

            if let Some(reassembler) = state.reassemblers.get_mut(&stream_id) {
                let delivered = reassembler.insert_with_digest(chunk_index, chunk.plaintext, chunk_digest);
                tracing::debug!(
                    stream_id, chunk_index,
                    delivered_count = delivered.len(),
                    "handle_bulk_decrypted: reassembler insert complete"
                );
                for (ci, data) in delivered {
                    credit_guard.release(data.len() as u64);
                    state.pending_bulk_deliveries.push((stream_id, ci, data));
                }
            } else {
                let chunks = state.early_bulk_chunks.entry(stream_id).or_default();
                if chunks.len() < 64 {
                    tracing::debug!(
                        stream_id, chunk_index,
                        buffered = chunks.len() + 1,
                        "handle_bulk_decrypted: buffered in early_bulk_chunks (no reassembler yet)"
                    );
                    chunks.push((chunk_index, chunk.plaintext, chunk_digest));
                } else {
                    crate::v4::bulk::counters::DIAG_EARLY_BULK_CHUNKS_DROPPED
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    tracing::error!(
                        stream_id, chunk_index,
                        buffered = chunks.len(),
                        "early_bulk_chunks: buffer full — chunk dropped, STREAM_OPEN never arrived"
                    );
                }
            }

            // FIN_FOLLOWS: single-chunk transfer
            if header_flags & crate::v4::wire::header::flags::FIN_FOLLOWS != 0 {
                tracing::debug!(stream_id, "handle_bulk_decrypted: FIN_FOLLOWS flag set — checking pending_fin_verify");
                if let Some(pending) = state.pending_fin_verify.remove(&stream_id) {
                    let hash_ok = state.reassemblers.get(&stream_id)
                        .map(|r| r.verify_content_hash(&pending.content_hash).is_ok())
                        .unwrap_or(false);

                    if hash_ok {
                        let total_bytes = state.reassemblers.get(&stream_id)
                            .map(|r| r.total_bytes()).unwrap_or(0);
                        let total_chunks = state.reassemblers.get(&stream_id)
                            .map(|r| r.next_expected()).unwrap_or(0);

                        tracing::debug!(
                            stream_id,
                            total_bytes, total_chunks,
                            transfer_id = %pending.transfer_id,
                            "handle_bulk_decrypted: FIN_FOLLOWS hash OK — emitting ACK"
                        );

                        let audit_link = audit_links.inbound_link.load();
                        let ack = crate::v4::codec::stream::ack::StreamAckPayload {
                            transfer_id: pending.transfer_id,
                            ack_byte_count: total_bytes,
                            ack_chunk_count: total_chunks,
                            audit_link,
                        };
                        state.pending_outbound.push(OutboundFrame::Data {
                            stream_id,
                            kind: StreamKind::Ack,
                            chunk_index: total_chunks,
                            payload: crate::v4::codec::stream::ack::encode(&ack),
                        });
                        state.pending_bulk_completions.push((
                            stream_id, pending.transfer_id, total_bytes as u64, total_chunks,
                        ));
                        router.on_bulk_complete(
                            info, stream_id, pending.transfer_id,
                            total_bytes, total_chunks,
                        );
                        state.reassemblers.remove(&stream_id);
                        let _ = state.stream_registry.close(stream_id, Direction::Inbound);
                    } else {
                        tracing::error!(
                            stream_id,
                            transfer_id = %pending.transfer_id,
                            "handle_bulk_decrypted: content hash mismatch on FIN_FOLLOWS"
                        );
                        router.on_bulk_failed(
                            info, stream_id, pending.transfer_id,
                            "content hash verification failed (FIN_FOLLOWS)",
                        );
                        state.reassemblers.remove(&stream_id);
                        let _ = state.stream_registry.close(stream_id, Direction::Inbound);
                        return Err(LaneShutdown);
                    }
                } else {
                    tracing::debug!(
                        stream_id,
                        "handle_bulk_decrypted: FIN_FOLLOWS but no pending_fin_verify — FIN not yet received"
                    );
                }
            } else {
                tracing::debug!(stream_id, "handle_bulk_decrypted: checking deferred FIN verify");
                check_deferred_fin_verify(state, router.as_ref(), info, audit_links);
            }

            tracing::debug!(
                stream_id,
                pending_deliveries = state.pending_bulk_deliveries.len(),
                "handle_bulk_decrypted: draining bulk deliveries"
            );
            drain_bulk_deliveries(state, bulk_data_tx).await?;
            tracing::debug!(stream_id, "handle_bulk_decrypted: bulk deliveries drained");

            tracing::debug!(
                stream_id,
                session_seq = inbound_link.session_seq,
                "handle_bulk_decrypted: forwarding inbound audit link to audit_merge"
            );
            frame_loop.audit_tx()
                .send(crate::v4::io::control_loop::audit_merge::AuditLinkDirection::Inbound(inbound_link))
                .await
                .map_err(|_| LaneShutdown)?;

            Ok(())
        }
        BulkDecryptResult::Fatal(e) => {
            tracing::error!(error = %e, "handle_bulk_decrypted: FATAL — session terminating");
            Err(LaneShutdown)
        }
    }
}

// ── Deferred FIN verification ───────────────────────────────────

fn check_deferred_fin_verify(
    state: &mut DataState,
    router: &dyn FrameRouter,
    info: &ConnectionInfo,
    audit_links: &SharedAuditLinks,
) {
    let stream_ids: Vec<u8> = state.pending_fin_verify.keys()
        .filter(|&&sid| state.reassemblers.contains_key(&sid))
        .copied()
        .collect();

    if stream_ids.is_empty() {
        tracing::debug!("check_deferred_fin_verify: no streams with both pending_fin_verify and reassembler");
        return;
    }

    tracing::debug!(
        candidate_count = stream_ids.len(),
        "check_deferred_fin_verify: evaluating candidates"
    );

    for stream_id in stream_ids {
        let next_expected = state.reassemblers.get(&stream_id)
            .map(|r| r.next_expected()).unwrap_or(0);
        let buffered = state.reassemblers.get(&stream_id)
            .map(|r| r.buffered_count()).unwrap_or(0);

        let expected_chunks = match state.pending_fin_verify.get(&stream_id) {
            Some(p) => p.expected_chunks,
            None => continue,
        };

        let ready = buffered == 0 && next_expected >= expected_chunks;
        tracing::debug!(
            stream_id,
            next_expected, buffered, expected_chunks, ready,
            "check_deferred_fin_verify: stream evaluation"
        );

        if !ready {
            continue;
        }

        let pending = match state.pending_fin_verify.remove(&stream_id) {
            Some(p) => p,
            None => continue,
        };

        let hash_ok = state.reassemblers.get(&stream_id)
            .map(|r| r.verify_content_hash(&pending.content_hash).is_ok())
            .unwrap_or(false);

        if hash_ok {
            let total_bytes = state.reassemblers.get(&stream_id)
                .map(|r| r.total_bytes()).unwrap_or(0);
            let total_chunks = state.reassemblers.get(&stream_id)
                .map(|r| r.next_expected()).unwrap_or(0);

            tracing::debug!(
                stream_id, total_bytes, total_chunks,
                transfer_id = %pending.transfer_id,
                "check_deferred_fin_verify: hash OK — emitting ACK"
            );

            let audit_link = audit_links.inbound_link.load();
            let ack = crate::v4::codec::stream::ack::StreamAckPayload {
                transfer_id: pending.transfer_id,
                ack_byte_count: total_bytes,
                ack_chunk_count: total_chunks,
                audit_link,
            };
            state.pending_outbound.push(OutboundFrame::Data {
                stream_id,
                kind: StreamKind::Ack,
                chunk_index: total_chunks,
                payload: crate::v4::codec::stream::ack::encode(&ack),
            });
            state.pending_bulk_completions.push((
                stream_id, pending.transfer_id, total_bytes as u64, total_chunks,
            ));
            router.on_bulk_complete(info, stream_id, pending.transfer_id, total_bytes, total_chunks);
            state.reassemblers.remove(&stream_id);
            let _ = state.stream_registry.close(stream_id, Direction::Inbound);
        } else {
            tracing::error!(
                stream_id,
                transfer_id = %pending.transfer_id,
                "check_deferred_fin_verify: content hash mismatch"
            );
            router.on_bulk_failed(info, stream_id, pending.transfer_id, "content hash verification failed");
            state.reassemblers.remove(&stream_id);
            let _ = state.stream_registry.close(stream_id, Direction::Inbound);
        }
    }
}

// ── FIN emission ────────────────────────────────────────────────

async fn emit_ready_fins(
    state: &mut DataState,
    frame_loop: &FrameLoop,
    audit_links: &SharedAuditLinks,
) -> Result<(), LaneShutdown> {
    let chain_next = audit_links.outbound_next_seq.load(std::sync::atomic::Ordering::Acquire);
    let stream_ids: Vec<u8> = state.pending_fins.keys().copied().collect();

    if stream_ids.is_empty() {
        tracing::debug!(chain_next, "emit_ready_fins: no pending FINs");
        return Ok(());
    }

    tracing::debug!(
        chain_next,
        pending_count = stream_ids.len(),
        "emit_ready_fins: checking FIN readiness"
    );

    for sid in stream_ids {
        let last_chunk_seq = state.pending_fins.get(&sid).map(|s| s.last_chunk_seq);
        let ready = state.pending_fins.get(&sid)
            .map(|s| chain_next > s.last_chunk_seq)
            .unwrap_or(false);

        tracing::debug!(
            stream_id = sid,
            chain_next,
            ?last_chunk_seq,
            ready,
            "emit_ready_fins: stream evaluation"
        );

        if ready {
            if let Some(completed) = state.pending_fins.remove(&sid) {
                let _ = state.stream_registry.transition(
                    sid,
                    Direction::Outbound,
                    crate::v4::stream::state::StreamEvent::FinSent,
                );

                let final_audit_link = audit_links.outbound_link.load();

                let fin_payload = crate::v4::codec::stream::fin::encode(
                    &crate::v4::codec::stream::fin::StreamFinPayload {
                        total_bytes: completed.total_bytes,
                        fault_count: 0,
                        final_content_hash: completed.content_hash,
                        final_audit_link,
                    },
                );

                let frame = OutboundFrame::Data {
                    stream_id: sid,
                    kind: StreamKind::Fin,
                    chunk_index: completed.chunk_count,
                    payload: fin_payload,
                };

                let encoder = Arc::clone(frame_loop.encoder());
                let ewa = encoder.encode_with_audit(&frame);
                let fin_link = ewa.link_input;
                let wire_bytes = ewa.encoded.into_wire_bytes();

                tracing::info!(
                    stream_id = sid,
                    chunk_count = completed.chunk_count,
                    total_bytes = completed.total_bytes,
                    last_chunk_seq = completed.last_chunk_seq,
                    fin_session_seq = fin_link.session_seq,
                    chain_next,
                    "emit_ready_fins: EMITTING STREAM_FIN"
                );

                tracing::debug!(
                    stream_id = sid,
                    fin_session_seq = fin_link.session_seq,
                    "emit_ready_fins: sending FIN audit link to audit_merge"
                );
                if frame_loop.audit_tx()
                    .send(crate::v4::io::control_loop::audit_merge::AuditLinkDirection::Outbound(fin_link))
                    .await
                    .is_err()
                {
                    tracing::error!(stream_id = sid, "emit_ready_fins: audit_tx closed — audit_merge dead");
                    return Err(LaneShutdown);
                }

                tracing::debug!(
                    stream_id = sid,
                    wire_len = wire_bytes.len(),
                    "emit_ready_fins: sending FIN wire bytes to bulk_wire_tx"
                );
                let bulk_wire_tx = completed.bulk_wire_tx.clone();
                if bulk_wire_tx.send(crate::v4::io::lane_channels::BulkFrame::Plain(wire_bytes)).is_err() {
                    tracing::error!(stream_id = sid, "emit_ready_fins: bulk_wire_tx closed — write task dead");
                    return Err(LaneShutdown);
                }
            }
        }
    }
    Ok(())
}

// ── Bulk delivery drain ─────────────────────────────────────────

async fn drain_bulk_deliveries(
    state: &mut DataState,
    bulk_data_tx: &mpsc::Sender<BulkDataSignal>,
) -> Result<(), LaneShutdown> {
    let delivery_count = state.pending_bulk_deliveries.len();
    let completion_count = state.pending_bulk_completions.len();
    if delivery_count > 0 || completion_count > 0 {
        tracing::debug!(
            deliveries = delivery_count,
            completions = completion_count,
            "drain_bulk_deliveries: draining"
        );
    }
    for (sid, ci, data) in std::mem::take(&mut state.pending_bulk_deliveries) {
        bulk_data_tx
            .send(BulkDataSignal::Chunk { stream_id: sid, chunk_index: ci, data })
            .await
            .map_err(|_| LaneShutdown)?;
    }
    for (sid, transfer_id, total_bytes, total_chunks) in std::mem::take(&mut state.pending_bulk_completions) {
        tracing::debug!(
            stream_id = sid,
            %transfer_id,
            total_bytes, total_chunks,
            "drain_bulk_deliveries: sending Complete signal"
        );
        bulk_data_tx
            .send(BulkDataSignal::Complete { stream_id: sid, transfer_id, total_bytes, total_chunks })
            .await
            .map_err(|_| LaneShutdown)?;
    }
    Ok(())
}

// ── Outbound wire frame drain ───────────────────────────────────

async fn drain_pending_outbound(
    state: &mut DataState,
    frame_loop: &mut FrameLoop,
) -> Result<(), LaneShutdown> {
    let count = state.pending_outbound.len();
    if count > 0 {
        tracing::debug!(frame_count = count, "drain_pending_outbound: draining");
    }
    for frame_out in std::mem::take(&mut state.pending_outbound) {
        let ewa = frame_loop.encoder().encode_with_audit(&frame_out);
        tracing::debug!(
            session_seq = ewa.link_input.session_seq,
            lane = ?ewa.lane,
            "drain_pending_outbound: encoding + sending outbound frame"
        );
        frame_loop.audit_tx()
            .send(crate::v4::io::control_loop::audit_merge::AuditLinkDirection::Outbound(ewa.link_input))
            .await
            .map_err(|_| LaneShutdown)?;
        frame_loop.lane_channels()
            .send_with_retry(ewa.lane, ewa.encoded.into_wire_bytes())
            .await
            .map_err(|_| LaneShutdown)?;
    }
    Ok(())
}

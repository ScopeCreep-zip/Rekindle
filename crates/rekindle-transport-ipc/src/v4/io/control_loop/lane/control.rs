//! Control lane task — Channel (0x01) + Datagram (0x03) frames,
//! heartbeat, pong timeout, sequenced outbound, client outbound.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::v4::bulk::counters::BulkCounters;
use crate::v4::codec::channel::ping as ping_codec;
use crate::v4::handlers::HandlerError;
use crate::v4::io::control_loop::heartbeat::HeartbeatState;
use crate::v4::io::control_loop::{ReadSignal, WriteError};
use crate::v4::io::read_task::SessionOutcome;
use crate::v4::router::{ConnectionInfo, ConnectionPhase, FrameRouter};
use crate::v4::session::state::{SessionEvent, SessionState};
use crate::v4::wire::frame_kind::ChannelKind;
use crate::v4::wire::outbound::{OutboundFrame, SequencedOutbound};

use super::state::ControlState;
use super::{extract_class_kind, FrameLoop};

pub async fn run(
    mut rx: mpsc::Receiver<ReadSignal>,
    mut state: ControlState,
    mut frame_loop: FrameLoop,
    router: Arc<dyn FrameRouter>,
    info: ConnectionInfo,
    mut write_error_rx: mpsc::Receiver<WriteError>,
    mut outbound_rx: mpsc::UnboundedReceiver<OutboundFrame>,
    mut sequenced_rx: mpsc::Receiver<SequencedOutbound>,
    counters: Arc<BulkCounters>,
    last_activity_ns: Arc<std::sync::atomic::AtomicU64>,
) -> SessionOutcome {
    let mut heartbeat = HeartbeatState::new(
        Duration::from_millis(state.config.heartbeat_interval_ms),
        counters,
        Arc::clone(&last_activity_ns),
    );

    let mut heartbeat_timer = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_millis(state.config.heartbeat_interval_ms),
        Duration::from_millis(state.config.heartbeat_interval_ms),
    );
    heartbeat_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    tracing::debug!("control lane: task started");

    loop {
        tokio::select! {
            biased;

            Some(e) = write_error_rx.recv() => {
                tracing::error!(error = ?e, "control lane: write task failed — fatal");
                return terminate(router.as_ref(), &info, &frame_loop, SessionOutcome::SubstrateReadFailed {
                    detail: format!("write task failed: {e:?}"),
                });
            }

            _ = &mut heartbeat.pong_sleep, if heartbeat.awaiting_pong() => {
                tracing::debug!(
                    miss_count = state.heartbeat_miss_count,
                    "control lane: pong timeout fired"
                );
                if let Some(outcome) = handle_pong_timeout(&mut heartbeat, &mut state, &frame_loop, router.as_ref(), &info) {
                    return outcome;
                }
            }

            _ = heartbeat_timer.tick() => {
                tracing::debug!("control lane: heartbeat tick");
                if let Some(outcome) = handle_heartbeat_tick(
                    &mut heartbeat, &mut state, &mut frame_loop, router.as_ref(), &info,
                ).await {
                    return outcome;
                }
            }

            signal = rx.recv() => {
                match signal {
                    Some(ReadSignal::Frame(frame)) => {
                        tracing::debug!(
                            session_seq = frame.envelope.session_seq,
                            "control lane: frame received"
                        );
                        heartbeat.record_activity();

                        if frame.plaintext.len() >= 2
                            && frame.plaintext[0] == 0x01
                            && frame.plaintext[1] == ChannelKind::Pong as u8
                        {
                            tracing::debug!(
                                session_seq = frame.envelope.session_seq,
                                "control lane: PONG intercepted"
                            );
                            let payload = &frame.plaintext[2..];
                            let was_degraded = state.heartbeat_miss_count > 0;
                            heartbeat.handle_pong_inline(&mut state, payload);
                            if was_degraded && state.heartbeat_miss_count == 0 {
                                tracing::debug!("control lane: heartbeat recovered — Degraded → Established");
                                crate::v4::io::control_loop::util::notify_state_change(
                                    router.as_ref(), &info,
                                    ConnectionPhase::Degraded, ConnectionPhase::Established,
                                );
                            }
                            let link = crate::v4::audit::chain::LinkInput {
                                session_seq: frame.envelope.session_seq,
                                envelope_hash: frame.audit_envelope_hash,
                                header_hash: frame.audit_header_hash,
                                ciphertext_hash: frame.audit_ciphertext_hash,
                            };
                            if frame_loop.drain_and_emit_audit(link, frame.retained_wire).await.is_err() {
                                tracing::debug!("control lane: audit_tx closed during PONG drain — exiting");
                                return SessionOutcome::ConnectionLost;
                            }
                            continue;
                        }

                        let shared = frame_loop.shared().clone();
                        let link = match frame_loop.process_frame(&frame, |f, outbound| {
                            let (class, kind, payload) = extract_class_kind(f)
                                .ok_or(HandlerError::CodecFailed("frame too short".into()))?;
                            crate::v4::dispatch::inbound::dispatch_control(
                                &mut state, &shared, router.as_ref(), &info,
                                outbound, class, kind, payload,
                            )
                        }) {
                            Ok(link) => link,
                            Err(e) => {
                                tracing::error!(
                                    error = ?e,
                                    session_seq = frame.envelope.session_seq,
                                    "control lane: handler error"
                                );
                                continue;
                            }
                        };

                        if frame_loop.drain_and_emit_audit(link, frame.retained_wire).await.is_err() {
                            tracing::debug!("control lane: audit_tx closed during frame drain — exiting");
                            return SessionOutcome::ConnectionLost;
                        }

                        if let Some(outcome) = check_deadlines(&mut state, &frame_loop, router.as_ref(), &info) {
                            return outcome;
                        }

                        let session_state = frame_loop.shared().read().session_state;
                        if session_state.is_terminal() {
                            tracing::debug!(?session_state, "control lane: terminal state reached");
                            return terminate(router.as_ref(), &info, &frame_loop, SessionOutcome::Closed {
                                peer_initiated: state.peer_final_session_seq.is_some(),
                            });
                        }
                    }
                    Some(ReadSignal::Finished(outcome)) => {
                        tracing::info!(?outcome, "control lane: read task finished");
                        let session_state = frame_loop.shared().read().session_state;
                        let final_outcome = match (&outcome, session_state) {
                            (SessionOutcome::ConnectionLost, SessionState::Draining) => {
                                tracing::debug!("control lane: ConnectionLost during Draining → treating as Closed");
                                SessionOutcome::Closed { peer_initiated: true }
                            }
                            _ => outcome,
                        };
                        return terminate(router.as_ref(), &info, &frame_loop, final_outcome);
                    }
                    None => {
                        tracing::debug!("control lane: rx channel closed — exiting");
                        return terminate(router.as_ref(), &info, &frame_loop, SessionOutcome::ConnectionLost);
                    }
                }
            }

            Some(seq) = sequenced_rx.recv() => {
                let has_completion = seq.completion.is_some();
                tracing::debug!(
                    has_completion,
                    frame_kind = ?seq.frame,
                    "control lane: sequenced outbound received"
                );
                if let Some(completion) = seq.completion {
                    state.pending_rotation_confirm = Some(completion);
                }
                let enc = Arc::clone(frame_loop.encoder());
                let ewa = enc.encode_with_audit(&seq.frame);
                let session_seq = ewa.link_input.session_seq;
                let wire = ewa.encoded.into_wire_bytes();
                let link = ewa.link_input;
                let lane = ewa.lane;
                tracing::debug!(
                    session_seq,
                    ?lane,
                    "control lane: sending sequenced outbound audit link"
                );
                frame_loop.audit_tx()
                    .send(crate::v4::io::control_loop::audit_merge::AuditLinkDirection::Outbound(link))
                    .await
                    .map_err(|_| ()).ok();
                if frame_loop.lane_channels()
                    .send_with_retry(lane, wire)
                    .await
                    .is_err()
                {
                    tracing::debug!("control lane: lane_channels closed during sequenced send — exiting");
                    return SessionOutcome::ConnectionLost;
                }
                tracing::debug!(session_seq, "control lane: sequenced outbound confirmed");
                let _ = seq.confirm.send(());
            }

            Some(frame) = outbound_rx.recv() => {
                tracing::debug!("control lane: client outbound received");
                if let OutboundFrame::Channel { kind: ChannelKind::Goodbye, .. } = &frame {
                    tracing::info!(
                        local_goodbye_already_sent = state.local_goodbye_sent,
                        "control lane: outbound GOODBYE from application"
                    );
                    if !state.local_goodbye_sent {
                        let mut s = frame_loop.shared().write();
                        let _ = s.session_state.apply(SessionEvent::GoodbyeSent);
                        s.shutting_down = true;
                        drop(s);
                        state.local_goodbye_sent = true;
                        state.drain_deadline = Some(std::time::Instant::now() + Duration::from_secs(5));
                        state.send_seq = frame_loop.encoder().current_session_seq();
                        tracing::info!(
                            send_seq = state.send_seq,
                            "control lane: GOODBYE state applied, drain deadline set"
                        );
                    }
                }

                let enc = Arc::clone(frame_loop.encoder());
                let ewa = enc.encode_with_audit(&frame);
                let session_seq = ewa.link_input.session_seq;
                let wire = ewa.encoded.into_wire_bytes();
                let link = ewa.link_input;
                let lane = ewa.lane;
                tracing::debug!(
                    session_seq,
                    ?lane,
                    "control lane: sending client outbound audit link"
                );
                frame_loop.audit_tx()
                    .send(crate::v4::io::control_loop::audit_merge::AuditLinkDirection::Outbound(link))
                    .await
                    .map_err(|_| ()).ok();
                if frame_loop.lane_channels()
                    .send_with_retry(lane, wire)
                    .await
                    .is_err()
                {
                    tracing::debug!("control lane: lane_channels closed during client send — exiting");
                    return SessionOutcome::ConnectionLost;
                }

                let session_state = frame_loop.shared().read().session_state;
                if session_state.is_terminal() {
                    tracing::debug!(?session_state, "control lane: terminal state after client outbound");
                    return terminate(router.as_ref(), &info, &frame_loop, SessionOutcome::Closed {
                        peer_initiated: state.peer_final_session_seq.is_some(),
                    });
                }
            }

            else => {
                tracing::debug!("control lane: all channels closed — exiting");
                return terminate(router.as_ref(), &info, &frame_loop, SessionOutcome::ConnectionLost);
            }
        }
    }
}

// ── Arm handler functions ───────────────────────────────────────

fn handle_pong_timeout(
    heartbeat: &mut HeartbeatState,
    state: &mut ControlState,
    frame_loop: &FrameLoop,
    router: &dyn FrameRouter,
    info: &ConnectionInfo,
) -> Option<SessionOutcome> {
    heartbeat.counters().heartbeat_pong_timeouts.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let was_degraded = state.heartbeat_miss_count > 0;

    if heartbeat.activity_elapsed() < Duration::from_millis(state.config.heartbeat_response_timeout_ms) {
        tracing::debug!(
            was_degraded,
            activity_elapsed_ms = heartbeat.activity_elapsed().as_millis() as u64,
            "control lane: pong timeout suppressed — peer activity within window"
        );
        state.heartbeat_miss_count = 0;
        state.previous_ping_nonce = state.last_ping_nonce.take();
        heartbeat.cancel_pong_timer();
        if was_degraded {
            crate::v4::io::control_loop::util::notify_state_change(
                router, info,
                ConnectionPhase::Degraded, ConnectionPhase::Established,
            );
        }
        return None;
    }

    let was_healthy = state.heartbeat_miss_count == 0;
    state.heartbeat_miss_count += 1;
    tracing::debug!(
        miss_count = state.heartbeat_miss_count,
        miss_limit = state.config.heartbeat_miss_limit,
        was_healthy,
        "control lane: pong timeout — miss counted"
    );

    if was_healthy {
        crate::v4::io::control_loop::util::notify_state_change(router, info, ConnectionPhase::Established, ConnectionPhase::Degraded);
    }

    state.previous_ping_nonce = state.last_ping_nonce.take();
    heartbeat.cancel_pong_timer();

    if state.heartbeat_miss_count >= state.config.heartbeat_miss_limit {
        tracing::error!(
            miss_count = state.heartbeat_miss_count,
            "control lane: heartbeat miss limit reached — terminating"
        );
        Some(terminate(router, info, frame_loop, SessionOutcome::HeartbeatTimeout {
            last_ping_at: crate::v4::io::control_loop::util::wall_ns(),
        }))
    } else {
        None
    }
}

async fn handle_heartbeat_tick(
    heartbeat: &mut HeartbeatState,
    state: &mut ControlState,
    frame_loop: &mut FrameLoop,
    router: &dyn FrameRouter,
    info: &ConnectionInfo,
) -> Option<SessionOutcome> {
    let interval_ms = state.config.heartbeat_interval_ms;
    if heartbeat.activity_elapsed() >= Duration::from_millis(interval_ms) && !heartbeat.awaiting_pong() {
        let nonce = crate::v4::io::control_loop::util::rand_nonce();
        let recv_last = state.audit_links.recv_last_seq.load(std::sync::atomic::Ordering::Acquire);
        tracing::debug!(
            nonce,
            recv_last_seq = recv_last,
            "control lane: emitting PING"
        );
        let ping = ping_codec::PingPayload {
            ping_nonce: nonce,
            sender_epoch_ns: crate::v4::io::control_loop::util::wall_ns(),
            last_seen_remote_seq: recv_last,
        };
        state.last_ping_nonce = Some(nonce);

        let frame = OutboundFrame::Channel {
            kind: ChannelKind::Ping,
            payload: ping_codec::encode(&ping),
        };
        let enc = Arc::clone(frame_loop.encoder());
        let ewa = enc.encode_with_audit(&frame);
        let ping_session_seq = ewa.link_input.session_seq;
        let wire = ewa.encoded.into_wire_bytes();
        let link = ewa.link_input;
        let lane = ewa.lane;
        tracing::debug!(
            session_seq = ping_session_seq,
            nonce,
            ?lane,
            "control lane: PING encoded, sending audit link"
        );
        frame_loop.audit_tx()
            .send(crate::v4::io::control_loop::audit_merge::AuditLinkDirection::Outbound(link))
            .await
            .map_err(|_| ()).ok();
        if frame_loop.lane_channels()
            .send_with_retry(lane, wire)
            .await
            .is_err()
        {
            tracing::debug!(session_seq = ping_session_seq, "control lane: lane_channels closed during PING send — exiting");
            return Some(SessionOutcome::ConnectionLost);
        }

        heartbeat.start_pong_timer(Duration::from_millis(state.config.heartbeat_response_timeout_ms));
        tracing::debug!(session_seq = ping_session_seq, nonce, "control lane: PING sent, pong timer started");
    }

    state.pending_requests.sweep_expired();

    check_deadlines(state, frame_loop, router, info)
}

fn check_deadlines(
    state: &mut ControlState,
    frame_loop: &FrameLoop,
    router: &dyn FrameRouter,
    info: &ConnectionInfo,
) -> Option<SessionOutcome> {
    let now = std::time::Instant::now();

    if let Some(deadline) = state.drain_deadline {
        if now >= deadline {
            tracing::info!("control lane: drain deadline reached — terminating");
            let mut s = frame_loop.shared().write();
            let _ = s.session_state.apply(SessionEvent::DrainTimeout);
            drop(s);
            return Some(terminate(router, info, frame_loop, SessionOutcome::DrainTimeout {
                local_goodbye_sent: state.local_goodbye_sent,
                peer_goodbye_received: state.peer_final_session_seq.is_some(),
                active_streams: 0,
            }));
        }
    }

    if let Some(deadline) = state.quiescence_deadline {
        if now >= deadline {
            tracing::info!("control lane: quiescence deadline reached — terminating");
            let mut s = frame_loop.shared().write();
            let _ = s.session_state.apply(SessionEvent::QuiescenceTimeout);
            drop(s);
            return Some(terminate(router, info, frame_loop, SessionOutcome::QuiescenceTimeout {
                duration_ms: 0,
            }));
        }
    }

    if let Some(deadline) = state.rotation_deadline {
        if now >= deadline {
            tracing::info!("control lane: rotation deadline reached — terminating");
            let mut s = frame_loop.shared().write();
            let _ = s.session_state.apply(SessionEvent::RotationTimeout);
            drop(s);
            return Some(terminate(router, info, frame_loop, SessionOutcome::RotationTimeout));
        }
    }

    None
}

fn terminate(
    router: &dyn FrameRouter,
    info: &ConnectionInfo,
    frame_loop: &FrameLoop,
    outcome: SessionOutcome,
) -> SessionOutcome {
    tracing::debug!(?outcome, "control lane: terminating");
    let session_state = frame_loop.shared().read().session_state;
    crate::v4::io::control_loop::util::terminate(router, info, session_state, outcome)
}

//! Handoff lane task — Streaming (0x05) frames.

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::v4::handlers::HandlerError;
use crate::v4::router::{ConnectionInfo, FrameRouter};

use super::{extract_class_kind, FrameLoop};
use super::state::HandoffState;
use crate::v4::io::control_loop::ReadSignal;

pub async fn run(
    mut rx: mpsc::Receiver<ReadSignal>,
    mut state: HandoffState,
    mut frame_loop: FrameLoop,
    router: Arc<dyn FrameRouter>,
    info: ConnectionInfo,
) {
    tracing::debug!(
        conn_id = state.conn_id,
        "handoff lane: task started"
    );
    #[cfg(target_os = "linux")]
    tracing::debug!(
        arena_count = state.arenas.len(),
        has_sidechannel = state.sidechannel.is_some(),
        has_pending_fds = state.pending_arena_fds.is_some(),
        "handoff lane: arena state"
    );

    while let Some(signal) = rx.recv().await {
        match signal {
            ReadSignal::Frame(frame) => {
                let frame_kind = if frame.plaintext.len() >= 2 { frame.plaintext[1] } else { 0xFF };
                tracing::debug!(
                    session_seq = frame.envelope.session_seq,
                    frame_kind,
                    plaintext_len = frame.plaintext.len(),
                    "handoff lane: frame received"
                );
                let link = match frame_loop.process_frame(&frame, |f, outbound| {
                    let (_, kind, payload) = extract_class_kind(f)
                        .ok_or(HandlerError::CodecFailed("frame too short".into()))?;
                    crate::v4::dispatch::inbound::dispatch_handoff(
                        &mut state, router.as_ref(), &info, outbound, kind, payload,
                    )
                }) {
                    Ok(link) => link,
                    Err(e) => {
                        tracing::error!(error = ?e, frame_kind, "handoff lane: handler error");
                        continue;
                    }
                };
                if frame_loop.drain_and_emit_audit(link, frame.retained_wire).await.is_err() {
                    tracing::debug!("handoff lane: audit_tx closed — exiting");
                    break;
                }
            }
            ReadSignal::Finished(outcome) => {
                tracing::info!(?outcome, "handoff lane: read task finished — shutting down");
                break;
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        tracing::debug!(arena_count = state.arenas.len(), "handoff lane: reclaiming in-flight arena slots");
        for arena in &state.arenas {
            arena.reclaim_all_inflight();
        }
    }
    tracing::debug!("handoff lane: task exiting");
}

//! Audit lane task — Audit (0x04) frames.

use tokio::sync::mpsc;

use crate::v4::handlers::HandlerError;

use super::{extract_class_kind, FrameLoop};
use super::state::AuditState;
use crate::v4::io::control_loop::ReadSignal;

pub async fn run(
    mut rx: mpsc::Receiver<ReadSignal>,
    mut state: AuditState,
    mut frame_loop: FrameLoop,
) {
    tracing::debug!("audit lane: task started");

    while let Some(signal) = rx.recv().await {
        match signal {
            ReadSignal::Frame(frame) => {
                let frame_kind = if frame.plaintext.len() >= 2 { frame.plaintext[1] } else { 0xFF };
                tracing::debug!(
                    session_seq = frame.envelope.session_seq,
                    frame_kind,
                    "audit lane: frame received"
                );
                let link = match frame_loop.process_frame(&frame, |f, outbound| {
                    let (_, kind, payload) = extract_class_kind(f)
                        .ok_or(HandlerError::CodecFailed("frame too short".into()))?;
                    crate::v4::dispatch::inbound::dispatch_audit(
                        &mut state, outbound, kind, payload,
                    )
                }) {
                    Ok(link) => link,
                    Err(e) => {
                        tracing::error!(error = ?e, frame_kind, "audit lane: handler error");
                        continue;
                    }
                };
                if frame_loop.drain_and_emit_audit(link, frame.retained_wire).await.is_err() {
                    tracing::debug!("audit lane: audit_tx closed — exiting");
                    break;
                }
            }
            ReadSignal::Finished(outcome) => {
                tracing::info!(?outcome, "audit lane: read task finished — shutting down");
                break;
            }
        }
    }
    tracing::debug!("audit lane: task exiting");
}

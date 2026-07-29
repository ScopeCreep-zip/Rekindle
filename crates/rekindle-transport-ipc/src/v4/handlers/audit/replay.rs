//! AUDIT_REPLAY handler — graft replayed frames into inbound chain.
//!
//! Retention storage happens locally (AuditState owns RetentionBuffer).
//! Chain grafting is sent to audit_merge via AuditLinkDirection::GraftInbound
//! because the inbound chain is owned exclusively by the merge task.
//!
//! Replay is a session-integrity operation. If the graft cannot reach
//! audit_merge, the inbound chain gap persists permanently. The peer
//! re-sends AUDIT_GAP indefinitely, burning bandwidth. The handler
//! returns Err(HandlerError) to terminate the session on graft failure.
//!
//! The handler does NOT echo replayed frames back to the peer — the peer
//! sent them, it already has them. Receipt confirmation is via the next
//! AUDIT_CHECKPOINT emitted by audit_merge (Spec 16).

use tokio::sync::mpsc;

use crate::v4::audit::retention::RetentionBuffer;
use crate::v4::codec::audit::replay as codec;
use crate::v4::handlers::HandlerError;
use crate::v4::io::control_loop::audit_merge::AuditLinkDirection;

pub fn handle(
    retention: &mut RetentionBuffer,
    audit_merge_tx: &mpsc::Sender<AuditLinkDirection>,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let replay = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    if replay.replay_frame_count == 0 {
        return Ok(());
    }

    let link_inputs = codec::extract_link_inputs(
        &replay.replayed_frames, replay.replay_frame_count,
    ).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let frame_bytes = codec::extract_frame_bytes(
        &replay.replayed_frames, replay.replay_frame_count,
    ).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    // Store retention — takes ownership of frame bytes, zero clones.
    for (seq, bytes) in link_inputs.iter().zip(frame_bytes.into_iter()) {
        retention.store(seq.session_seq, bytes);
    }

    // Graft into inbound chain via audit_merge — session-fatal if it fails.
    if audit_merge_tx.try_send(AuditLinkDirection::GraftInbound {
        link_inputs,
    }).is_err() {
        tracing::error!("replay: audit_merge_tx full — graft lost, session must terminate");
        return Err(HandlerError::CodecFailed("audit merge channel full during replay graft".into()));
    }

    Ok(())
}

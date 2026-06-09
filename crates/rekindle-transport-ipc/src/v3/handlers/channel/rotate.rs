use std::time::{Duration, Instant};

use crate::v3::codec::channel::rotate as codec;
use crate::v3::context::{OutboundFrame, OutboundChannelKind, OutboundAuditKind, SessionContext};
use crate::v3::handlers::HandlerError;
use crate::v3::session::handshake;
use crate::v3::session::state::SessionEvent;

/// Responder: receives ROTATE_INIT, derives epoch=1 keys, sends ROTATE_COMMIT.
///
/// Critical ordering:
/// 1. Derive epoch=1 decoder keys AND encoder keys (separate directions)
/// 2. Install epoch=1 DECODER keys via epoch_signal (read task can decode
///    the initiator's post-rotation frames when they arrive)
/// 3. Push COMMIT to outbound queue (encoded with epoch=0 by drain_outbound)
/// 4. Transition Rotating → Established via RotateCommitSent
/// 5. Store epoch=1 ENCODER keys as deferred — installed by the control loop
///    when the initiator's first epoch=1 frame arrives (peer_epoch_advanced)
///
/// The responder does NOT swap its encoder immediately. It continues encoding
/// with epoch=0 until the initiator proves it has swapped. This breaks the
/// mutual-deadlock where both sides wait for the other to send epoch=1 first.
pub fn handle_init(ctx: &mut SessionContext, payload: &[u8]) -> Result<(), HandlerError> {
    tracing::info!(state = ?ctx.session_state(), "handle_init: ENTER (responder)");

    let init = codec::decode_init(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    ctx.session_state_mut()
        .apply(SessionEvent::RotateInitReceived)
        .map_err(|_| HandlerError::FrameDisallowedInState)?;

    ctx.set_rotation_deadline(
        Instant::now() + Duration::from_millis(init.phase_2_deadline_ms)
    );

    let transcript_anchor = ctx.inbound_chain().current_link();
    let commit = ctx.rotation_mut().receive_init(&init, transcript_anchor)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    // Derive BOTH decoder and encoder keys via handshake SSOT.
    // Direction mapping lives in handshake.rs — handlers never import wire constants.
    let new_keys = ctx.rotation_mut().current_keys().clone();
    let role = ctx.role();
    let aead_code = ctx.agreed_aead();
    let decoder_keys = handshake::build_epoch_keys(&new_keys, role, aead_code, false)
        .map_err(|e| HandlerError::CodecFailed(format!("decoder keys: {e:?}")))?;
    let encoder_keys = handshake::build_epoch_keys(&new_keys, role, aead_code, true)
        .map_err(|e| HandlerError::CodecFailed(format!("encoder keys: {e:?}")))?;

    // 1. Queue BOTH decoder and encoder keys for the read task.
    //    The read task installs both atomically before reading the next
    //    frame — no cross-thread race, no signal timing dependency.
    //    The encoder install happens AFTER drain_outbound encodes
    //    ROTATE_COMMIT with old keys (the read task drains the signal
    //    after blocking_send, which is after drain_outbound).
    ctx.install_next_epoch_keys(decoder_keys, encoder_keys);
    tracing::debug!("handle_init: both decoder + encoder keys queued for read task");

    // 2. Push COMMIT. drain_outbound encodes it with old-epoch keys
    //    because the encoder hasn't been swapped yet (the read task
    //    installs the encoder keys after this frame is processed).
    ctx.push_outbound(OutboundFrame::Channel {
        kind: OutboundChannelKind::RotateCommit,
        payload: codec::encode_commit(&commit),
    });

    // 3. Transition to Established. Responder rotation handshake done.
    ctx.session_state_mut()
        .apply(SessionEvent::RotateCommitSent)
        .map_err(|_| HandlerError::FrameDisallowedInState)?;

    ctx.clear_rotation_deadline();

    tracing::info!(state = ?ctx.session_state(), "handle_init: EXIT (responder → Established, encoder immediate)");
    Ok(())
}

/// Initiator: receives ROTATE_COMMIT, derives epoch=1 keys.
///
/// Critical ordering:
/// 1. Derive epoch=1 decoder keys AND encoder keys
/// 2. Install epoch=1 DECODER keys via epoch_signal
/// 3. Store epoch=1 ENCODER keys as immediate — the control loop installs
///    them right after drain_outbound on this same select! iteration.
///    The initiator is the first to send epoch=1 frames, breaking the deadlock.
/// 4. Resolve the rotation completion oneshot
pub fn handle_commit(ctx: &mut SessionContext, payload: &[u8]) -> Result<(), HandlerError> {
    tracing::info!(state = ?ctx.session_state(), "handle_commit: ENTER (initiator)");

    let commit = codec::decode_commit(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    ctx.rotation_mut().receive_commit(&commit)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let new_keys = ctx.rotation_mut().current_keys().clone();
    let role = ctx.role();
    let aead_code = ctx.agreed_aead();
    let decoder_keys = handshake::build_epoch_keys(&new_keys, role, aead_code, false)
        .map_err(|e| HandlerError::CodecFailed(format!("decoder keys: {e:?}")))?;
    let encoder_keys = handshake::build_epoch_keys(&new_keys, role, aead_code, true)
        .map_err(|e| HandlerError::CodecFailed(format!("encoder keys: {e:?}")))?;

    // 1. Queue BOTH decoder and encoder keys for the read task.
    ctx.install_next_epoch_keys(decoder_keys, encoder_keys);
    tracing::debug!("handle_commit: both decoder + encoder keys queued for read task");

    ctx.clear_rotation_deadline();

    ctx.session_state_mut()
        .apply(SessionEvent::RotateCommitValid)
        .map_err(|_| HandlerError::FrameDisallowedInState)?;

    // 3. Resolve the rotation completion oneshot — rotate_keys() awaits this.
    if let Some(tx) = ctx.take_pending_rotation_confirm() {
        let _ = tx.send(Ok(()));
    }

    // 4. Audit checkpoint bridging the rotation
    let cp_payload = crate::v3::codec::audit::checkpoint::encode(
        &crate::v3::codec::audit::checkpoint::AuditCheckpointPayload {
            chain_index: 0,
            chain_length: ctx.outbound_chain().length(),
            checkpoint_seq: 0,
            wall_clock_ns: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0),
            chain_link: ctx.outbound_chain().current_link(),
            anchor_link: ctx.outbound_chain().anchor_record().value,
        }
    );
    ctx.push_outbound(OutboundFrame::Audit {
        kind: OutboundAuditKind::Checkpoint,
        payload: cp_payload,
    });

    tracing::info!(state = ?ctx.session_state(), "handle_commit: EXIT (initiator → Established, encoder immediate)");
    Ok(())
}

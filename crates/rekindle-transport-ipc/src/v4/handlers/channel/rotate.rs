//! Key rotation handlers — two-phase ROTATE_INIT / ROTATE_COMMIT.
//!
//! Rotation is a session-integrity operation. If the rotation anchor
//! cannot be delivered to audit_merge, the audit chain is permanently
//! divergent. The handler returns Err(HandlerError) which terminates
//! the session. Clean failure is better than silent chain corruption.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::v4::codec::channel::rotate as codec;
use crate::v4::handlers::HandlerError;
use crate::v4::io::control_loop::audit_merge::AuditLinkDirection;
use crate::v4::io::control_loop::shared_state::{SessionStateHandle, SharedAuditLinks};
use crate::v4::io::epoch_signal::EpochSignal;
use crate::v4::session::handshake;
use crate::v4::session::rotation::{RotationCoordinator, RotationError};
use crate::v4::session::state::SessionEvent;
use crate::v4::session::SessionRole;
use crate::v4::wire::outbound::{OutboundFrame, OutboundChannelKind, OutboundAuditKind};

/// Responder: receives ROTATE_INIT, derives epoch=1 keys, sends ROTATE_COMMIT.
pub fn handle_init(
    shared: &SessionStateHandle,
    outbound: &mut Vec<OutboundFrame>,
    rotation: &mut RotationCoordinator,
    rotation_deadline: &mut Option<Instant>,
    signal_epoch: &mut u8,
    epoch_signal: &Arc<EpochSignal>,
    audit_links: &SharedAuditLinks,
    audit_merge_tx: &mpsc::Sender<AuditLinkDirection>,
    role: SessionRole,
    agreed_aead: u8,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let session_state = shared.read().session_state;
    tracing::info!(?session_state, "handle_init: ENTER (responder)");

    {
        let mut s = shared.write();
        s.session_state
            .apply(SessionEvent::RotateInitReceived)
            .map_err(|_| HandlerError::FrameDisallowedInState)?;
    }

    let init = codec::decode_init(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    *rotation_deadline = Some(Instant::now() + Duration::from_millis(init.phase_2_deadline_ms));

    let transcript_anchor = audit_links.inbound_link.load();
    let commit = rotation.receive_init(&init, transcript_anchor)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let new_keys = rotation.current_keys().clone();
    let decoder_keys = handshake::build_epoch_keys(&new_keys, role, agreed_aead, false)
        .map_err(|e| HandlerError::CodecFailed(format!("decoder keys: {e:?}")))?;
    let encoder_keys = handshake::build_epoch_keys(&new_keys, role, agreed_aead, true)
        .map_err(|e| HandlerError::CodecFailed(format!("encoder keys: {e:?}")))?;

    let next_epoch = *signal_epoch ^ 1;
    *signal_epoch = next_epoch;
    epoch_signal.send_install(next_epoch, decoder_keys, encoder_keys);

    outbound.push(OutboundFrame::Channel {
        kind: OutboundChannelKind::RotateCommit,
        payload: codec::encode_commit(&commit),
    });

    {
        let mut s = shared.write();
        s.session_state
            .apply(SessionEvent::RotateCommitSent)
            .map_err(|_| HandlerError::FrameDisallowedInState)?;
    }

    *rotation_deadline = None;

    // Reset audit chains for the new epoch via audit_merge.
    // rotation_link MUST succeed — a None means the rotation is
    // cryptographically incomplete. A zero anchor would let an
    // attacker who compromised old keys forge post-rotation chain entries.
    let rotation_link = rotation.rotation_link(
        &audit_links.outbound_link.load(),
    ).ok_or_else(|| HandlerError::CodecFailed(
        "rotation_link computation failed — rotation incomplete, session must terminate".into(),
    ))?;

    let (outbound_audit_key, inbound_audit_key) = match role {
        SessionRole::Dialler => (new_keys.audit_d2l, new_keys.audit_l2d),
        SessionRole::Listener => (new_keys.audit_l2d, new_keys.audit_d2l),
    };

    // Rotation anchor delivery is session-fatal if it fails.
    // A rotation without an anchor means the audit chain is permanently
    // divergent — keys rotated but provenance is broken.
    if audit_merge_tx.try_send(AuditLinkDirection::RotationAnchor {
        outbound_key: outbound_audit_key,
        inbound_key: inbound_audit_key,
        rotation_link,
        start_session_seq: 0,
    }).is_err() {
        tracing::error!("rotate handle_init: audit_merge_tx full — rotation anchor lost, session must terminate");
        return Err(HandlerError::CodecFailed("audit merge channel full during rotation — anchor lost".into()));
    }

    let session_state = shared.read().session_state;
    tracing::info!(?session_state, "handle_init: EXIT (responder → Established)");
    Ok(())
}

/// Initiator: receives ROTATE_COMMIT, derives epoch=1 keys.
pub fn handle_commit(
    shared: &SessionStateHandle,
    outbound: &mut Vec<OutboundFrame>,
    rotation: &mut RotationCoordinator,
    rotation_deadline: &mut Option<Instant>,
    pending_rotation_confirm: &mut Option<tokio::sync::oneshot::Sender<Result<(), RotationError>>>,
    signal_epoch: &mut u8,
    epoch_signal: &Arc<EpochSignal>,
    audit_links: &SharedAuditLinks,
    audit_merge_tx: &mpsc::Sender<AuditLinkDirection>,
    role: SessionRole,
    agreed_aead: u8,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let session_state = shared.read().session_state;
    tracing::info!(?session_state, "handle_commit: ENTER (initiator)");

    let commit = codec::decode_commit(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    rotation.receive_commit(&commit)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let new_keys = rotation.current_keys().clone();
    let decoder_keys = handshake::build_epoch_keys(&new_keys, role, agreed_aead, false)
        .map_err(|e| HandlerError::CodecFailed(format!("decoder keys: {e:?}")))?;
    let encoder_keys = handshake::build_epoch_keys(&new_keys, role, agreed_aead, true)
        .map_err(|e| HandlerError::CodecFailed(format!("encoder keys: {e:?}")))?;

    let next_epoch = *signal_epoch ^ 1;
    *signal_epoch = next_epoch;
    epoch_signal.send_install(next_epoch, decoder_keys, encoder_keys);

    *rotation_deadline = None;

    {
        let mut s = shared.write();
        s.session_state
            .apply(SessionEvent::RotateCommitValid)
            .map_err(|_| HandlerError::FrameDisallowedInState)?;
    }

    if let Some(tx) = pending_rotation_confirm.take() {
        let _ = tx.send(Ok(()));
    }

    // Reset audit chains for the new epoch via audit_merge.
    let rotation_link = rotation.rotation_link(
        &audit_links.outbound_link.load(),
    ).ok_or_else(|| HandlerError::CodecFailed(
        "rotation_link computation failed — rotation incomplete, session must terminate".into(),
    ))?;

    let (outbound_audit_key, inbound_audit_key) = match role {
        SessionRole::Dialler => (new_keys.audit_d2l, new_keys.audit_l2d),
        SessionRole::Listener => (new_keys.audit_l2d, new_keys.audit_d2l),
    };

    if audit_merge_tx.try_send(AuditLinkDirection::RotationAnchor {
        outbound_key: outbound_audit_key,
        inbound_key: inbound_audit_key,
        rotation_link,
        start_session_seq: 0,
    }).is_err() {
        tracing::error!("rotate handle_commit: audit_merge_tx full — rotation anchor lost, session must terminate");
        return Err(HandlerError::CodecFailed("audit merge channel full during rotation — anchor lost".into()));
    }

    // Rotation-bridging checkpoint — ensures the peer has a checkpoint
    // that spans the rotation boundary. This is a special-case emission,
    // not a cadence checkpoint. audit_merge's cadence checkpoints may not
    // fire immediately after rotation.
    let outbound_link = audit_links.outbound_link.load();
    let cp_payload = crate::v4::codec::audit::checkpoint::encode(
        &crate::v4::codec::audit::checkpoint::AuditCheckpointPayload {
            chain_index: 0,
            chain_length: 0,
            checkpoint_seq: 0,
            wall_clock_ns: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0),
            chain_link: outbound_link,
            anchor_link: outbound_link,
        },
    );
    outbound.push(OutboundFrame::Audit {
        kind: OutboundAuditKind::Checkpoint,
        payload: cp_payload,
    });

    let session_state = shared.read().session_state;
    tracing::info!(?session_state, "handle_commit: EXIT (initiator → Established)");
    Ok(())
}

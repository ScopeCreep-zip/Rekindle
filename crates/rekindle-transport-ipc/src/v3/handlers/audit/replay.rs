use crate::v3::audit::replay::graft_frames;
use crate::v3::codec::audit::replay as codec;
use crate::v3::context::{OutboundFrame, OutboundAuditKind, SessionContext};
use crate::v3::handlers::HandlerError;

pub fn handle(ctx: &mut SessionContext, payload: &[u8]) -> Result<(), HandlerError> {
    let replay = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    if replay.replay_frame_count == 0 {
        return Ok(());
    }

    let link_inputs = codec::extract_link_inputs(&replay.replayed_frames, replay.replay_frame_count)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    graft_frames(ctx.inbound_chain_mut(), &link_inputs, &[]);

    let frame_bytes = codec::extract_frame_bytes(&replay.replayed_frames, replay.replay_frame_count)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;
    for (seq, bytes) in link_inputs.iter().zip(frame_bytes.iter()) {
        ctx.retention_mut().store(seq.session_seq, bytes.clone());
    }

    ctx.push_outbound(OutboundFrame::Audit {
        kind: OutboundAuditKind::Replay,
        payload: replay.replayed_frames,
    });

    Ok(())
}

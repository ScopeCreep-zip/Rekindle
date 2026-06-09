use crate::v3::audit::gap::MissingBitmap;
use crate::v3::codec::audit::gap as gap_codec;
use crate::v3::codec::audit::replay as replay_codec;
use crate::v3::context::{OutboundFrame, OutboundAuditKind, SessionContext};
use crate::v3::handlers::HandlerError;

pub fn handle(ctx: &mut SessionContext, payload: &[u8]) -> Result<(), HandlerError> {
    let gap = gap_codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let bitmap = MissingBitmap::from_bytes(
        gap.gap_start_seq,
        gap.gap_end_seq,
        &gap.missing_bitmap,
    );

    let missing_seqs: Vec<u64> = bitmap.missing_seqs().collect();

    let mut replayed_frames = Vec::new();
    let mut found_count = 0u32;
    let mut total_bytes = 0u32;

    for &seq in &missing_seqs {
        if let Some(frame_bytes) = ctx.retention().get(seq) {
            total_bytes += u32::try_from(frame_bytes.len()).expect("frame exceeds u32");
            found_count += 1;
            replayed_frames.extend_from_slice(frame_bytes);
        }
    }

    let replay = replay_codec::AuditReplayPayload {
        gap_id: gap.gap_id,
        replay_frame_count: found_count,
        replay_total_bytes: total_bytes,
        replayed_frames,
    };

    ctx.push_outbound(OutboundFrame::Audit {
        kind: OutboundAuditKind::Replay,
        payload: replay_codec::encode(&replay),
    });

    Ok(())
}

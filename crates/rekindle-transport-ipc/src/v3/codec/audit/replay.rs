#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditReplayPayload {
    pub gap_id: uuid::Uuid,
    pub replay_frame_count: u32,
    pub replay_total_bytes: u32,
    pub replayed_frames: Vec<u8>,
}

pub fn encode(p: &AuditReplayPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(32 + p.replayed_frames.len());
    buf.extend_from_slice(p.gap_id.as_bytes());                        // 0..16
    buf.extend_from_slice(&p.replay_frame_count.to_le_bytes());        // 16..20
    buf.extend_from_slice(&p.replay_total_bytes.to_le_bytes());        // 20..24
    buf.extend_from_slice(&[0u8; 8]);                                  // 24..32 reserved
    buf.extend_from_slice(&p.replayed_frames);                         // 32..
    buf
}

pub fn decode(buf: &[u8]) -> Result<AuditReplayPayload, CodecError> {
    if buf.len() < 32 {
        return Err(CodecError::TooShort { expected: 32, got: buf.len() });
    }
    let gap_id = uuid::Uuid::from_bytes(buf[0..16].try_into().unwrap());
    let replay_frame_count = u32::from_le_bytes(buf[16..20].try_into().unwrap());
    let replay_total_bytes = u32::from_le_bytes(buf[20..24].try_into().unwrap());
    let replayed_frames = buf[32..].to_vec();

    Ok(AuditReplayPayload { gap_id, replay_frame_count, replay_total_bytes, replayed_frames })
}

/// Extract LinkInputs from the raw replayed frame bytes.
/// Each frame is an Envelope (32 bytes) + Body (body_len bytes).
/// Returns LinkInputs for chain grafting.
pub fn extract_link_inputs(
    replayed_frames: &[u8],
    frame_count: u32,
) -> Result<Vec<crate::v3::audit::chain::LinkInput>, CodecError> {
    let mut inputs = Vec::with_capacity(frame_count as usize);
    let mut offset = 0;

    for i in 0..frame_count {
        if offset + 32 > replayed_frames.len() {
            return Err(CodecError::TruncatedFrame { index: i, offset });
        }

        let body_len = u32::from_le_bytes(
            replayed_frames[offset + 4..offset + 8].try_into()
                .map_err(|_| CodecError::TruncatedFrame { index: i, offset })?
        ) as usize;

        let session_seq = u64::from_le_bytes(
            replayed_frames[offset + 8..offset + 16].try_into()
                .map_err(|_| CodecError::TruncatedFrame { index: i, offset })?
        );

        let frame_end = offset + 32 + body_len;
        if frame_end > replayed_frames.len() {
            return Err(CodecError::TruncatedFrame { index: i, offset });
        }

        let envelope_bytes = &replayed_frames[offset..offset + 32];
        let body_bytes = &replayed_frames[offset + 32..frame_end];

        let envelope_hash = *blake3::hash(envelope_bytes).as_bytes();
        let ciphertext_hash = *blake3::hash(body_bytes).as_bytes();

        let lane = envelope_bytes[1];
        let header_hash = if lane == 0x02 && body_bytes.len() >= 32 {
            *blake3::hash(&body_bytes[..32]).as_bytes()
        } else {
            [0u8; 32]
        };

        inputs.push(crate::v3::audit::chain::LinkInput {
            session_seq,
            envelope_hash,
            header_hash,
            ciphertext_hash,
        });

        offset = frame_end;
    }

    Ok(inputs)
}

/// Split replayed_frames into individual frame byte vectors.
pub fn extract_frame_bytes(
    replayed_frames: &[u8],
    frame_count: u32,
) -> Result<Vec<Vec<u8>>, CodecError> {
    let mut frames = Vec::with_capacity(frame_count as usize);
    let mut offset = 0;

    for i in 0..frame_count {
        if offset + 32 > replayed_frames.len() {
            return Err(CodecError::TruncatedFrame { index: i, offset });
        }
        let body_len = u32::from_le_bytes(
            replayed_frames[offset + 4..offset + 8].try_into()
                .map_err(|_| CodecError::TruncatedFrame { index: i, offset })?
        ) as usize;
        let frame_end = offset + 32 + body_len;
        if frame_end > replayed_frames.len() {
            return Err(CodecError::TruncatedFrame { index: i, offset });
        }
        frames.push(replayed_frames[offset..frame_end].to_vec());
        offset = frame_end;
    }

    Ok(frames)
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
    TruncatedFrame { index: u32, offset: usize },
}

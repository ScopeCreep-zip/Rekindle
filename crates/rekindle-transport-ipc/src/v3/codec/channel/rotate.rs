//! CHANNEL_ROTATE_INIT and CHANNEL_ROTATE_COMMIT codec.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RotateInitPayload {
    pub rotation_id: uuid::Uuid,
    pub initiator_generation: u64,
    pub phase_2_deadline_ms: u64,
    pub new_static_pub: [u8; 32],
    pub rotation_chain_secret: [u8; 32],
    pub transcript_anchor: [u8; 32],
}

pub fn encode_init(p: &RotateInitPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(128);
    buf.extend_from_slice(p.rotation_id.as_bytes());                   // 0..16
    buf.extend_from_slice(&p.initiator_generation.to_le_bytes());      // 16..24
    buf.extend_from_slice(&p.phase_2_deadline_ms.to_le_bytes());       // 24..32
    buf.extend_from_slice(&p.new_static_pub);                          // 32..64
    buf.extend_from_slice(&p.rotation_chain_secret);                   // 64..96
    buf.extend_from_slice(&p.transcript_anchor);                       // 96..128
    buf
}

pub fn decode_init(buf: &[u8]) -> Result<RotateInitPayload, CodecError> {
    if buf.len() < 128 {
        return Err(CodecError::TooShort { expected: 128, got: buf.len() });
    }
    let rotation_id = uuid::Uuid::from_bytes(buf[0..16].try_into().unwrap());
    let initiator_generation = u64::from_le_bytes(buf[16..24].try_into().unwrap());
    let phase_2_deadline_ms = u64::from_le_bytes(buf[24..32].try_into().unwrap());
    let mut new_static_pub = [0u8; 32];
    new_static_pub.copy_from_slice(&buf[32..64]);
    let mut rotation_chain_secret = [0u8; 32];
    rotation_chain_secret.copy_from_slice(&buf[64..96]);
    let mut transcript_anchor = [0u8; 32];
    transcript_anchor.copy_from_slice(&buf[96..128]);

    Ok(RotateInitPayload {
        rotation_id, initiator_generation, phase_2_deadline_ms,
        new_static_pub, rotation_chain_secret, transcript_anchor,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RotateCommitPayload {
    pub rotation_id: uuid::Uuid,
    pub responder_generation: u64,
    pub new_static_pub: [u8; 32],
    pub rotation_chain_secret: [u8; 32],
    pub transcript_anchor: [u8; 32],
}

pub fn encode_commit(p: &RotateCommitPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(128);
    buf.extend_from_slice(p.rotation_id.as_bytes());                   // 0..16
    buf.extend_from_slice(&p.responder_generation.to_le_bytes());      // 16..24
    buf.extend_from_slice(&[0u8; 8]);                                  // 24..32 reserved
    buf.extend_from_slice(&p.new_static_pub);                          // 32..64
    buf.extend_from_slice(&p.rotation_chain_secret);                   // 64..96
    buf.extend_from_slice(&p.transcript_anchor);                       // 96..128
    buf
}

pub fn decode_commit(buf: &[u8]) -> Result<RotateCommitPayload, CodecError> {
    if buf.len() < 128 {
        return Err(CodecError::TooShort { expected: 128, got: buf.len() });
    }
    let rotation_id = uuid::Uuid::from_bytes(buf[0..16].try_into().unwrap());
    let responder_generation = u64::from_le_bytes(buf[16..24].try_into().unwrap());
    let mut new_static_pub = [0u8; 32];
    new_static_pub.copy_from_slice(&buf[32..64]);
    let mut rotation_chain_secret = [0u8; 32];
    rotation_chain_secret.copy_from_slice(&buf[64..96]);
    let mut transcript_anchor = [0u8; 32];
    transcript_anchor.copy_from_slice(&buf[96..128]);

    Ok(RotateCommitPayload {
        rotation_id, responder_generation,
        new_static_pub, rotation_chain_secret, transcript_anchor,
    })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
}

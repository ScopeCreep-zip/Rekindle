use crate::v3::wire::capability::CapabilityBits;
use crate::v3::wire::clearance::Clearance;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelloAckPayload {
    pub session_id: uuid::Uuid,
    pub listener_peer_id: [u8; 32],
    pub capabilities: CapabilityBits,
    pub agreed_clearance: Clearance,
    pub agreed_aead: u8,
    pub listener_epoch_ns: u64,
    pub listener_handshake_hash: [u8; 32],
}

pub fn encode(p: &HelloAckPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(104);
    buf.extend_from_slice(p.session_id.as_bytes());                // 0..16
    buf.extend_from_slice(&p.listener_peer_id);                    // 16..48
    buf.extend_from_slice(&p.capabilities.bits().to_le_bytes());   // 48..56
    buf.push(p.agreed_clearance as u8);                            // 56
    buf.push(p.agreed_aead);                                       // 57
    buf.extend_from_slice(&[0u8; 6]);                              // 58..64 reserved
    buf.extend_from_slice(&p.listener_epoch_ns.to_le_bytes());     // 64..72
    buf.extend_from_slice(&p.listener_handshake_hash);             // 72..104
    buf
}

pub fn decode(buf: &[u8]) -> Result<HelloAckPayload, CodecError> {
    if buf.len() < 104 {
        return Err(CodecError::TooShort { expected: 104, got: buf.len() });
    }
    let session_id = uuid::Uuid::from_bytes(buf[0..16].try_into().unwrap());
    let mut listener_peer_id = [0u8; 32];
    listener_peer_id.copy_from_slice(&buf[16..48]);
    let cap_bits = u64::from_le_bytes(buf[48..56].try_into().unwrap());
    let capabilities = CapabilityBits::from_bits_retain(cap_bits);
    let agreed_clearance =
        Clearance::try_from(buf[56]).map_err(|e| CodecError::InvalidClearance(e.0))?;
    let agreed_aead = buf[57];
    let listener_epoch_ns = u64::from_le_bytes(buf[64..72].try_into().unwrap());
    let mut listener_handshake_hash = [0u8; 32];
    listener_handshake_hash.copy_from_slice(&buf[72..104]);

    Ok(HelloAckPayload {
        session_id,
        listener_peer_id,
        capabilities,
        agreed_clearance,
        agreed_aead,
        listener_epoch_ns,
        listener_handshake_hash,
    })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
    InvalidClearance(u8),
}

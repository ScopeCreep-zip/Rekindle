use crate::v3::wire::capability::CapabilityBits;
use crate::v3::wire::clearance::Clearance;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelloPayload {
    pub session_id_proposal: uuid::Uuid,
    pub dialler_peer_id: [u8; 32],
    pub capabilities: CapabilityBits,
    pub proposed_clearance: Clearance,
    pub dialler_epoch_ns: u64,
    pub dialler_handshake_hash: [u8; 32],
}

pub fn encode(p: &HelloPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(104);
    buf.extend_from_slice(p.session_id_proposal.as_bytes());       // 0..16
    buf.extend_from_slice(&p.dialler_peer_id);                     // 16..48
    buf.extend_from_slice(&p.capabilities.bits().to_le_bytes());   // 48..56
    buf.push(p.proposed_clearance as u8);                          // 56
    buf.extend_from_slice(&[0u8; 7]);                              // 57..64 reserved
    buf.extend_from_slice(&p.dialler_epoch_ns.to_le_bytes());      // 64..72
    buf.extend_from_slice(&p.dialler_handshake_hash);              // 72..104
    buf
}

pub fn decode(buf: &[u8]) -> Result<HelloPayload, CodecError> {
    if buf.len() < 104 {
        return Err(CodecError::TooShort { expected: 104, got: buf.len() });
    }
    let session_id_proposal = uuid::Uuid::from_bytes(buf[0..16].try_into().unwrap());
    let mut dialler_peer_id = [0u8; 32];
    dialler_peer_id.copy_from_slice(&buf[16..48]);
    let cap_bits = u64::from_le_bytes(buf[48..56].try_into().unwrap());
    let capabilities = CapabilityBits::from_bits_retain(cap_bits);
    let proposed_clearance =
        Clearance::try_from(buf[56]).map_err(|e| CodecError::InvalidClearance(e.0))?;
    let dialler_epoch_ns = u64::from_le_bytes(buf[64..72].try_into().unwrap());
    let mut dialler_handshake_hash = [0u8; 32];
    dialler_handshake_hash.copy_from_slice(&buf[72..104]);

    Ok(HelloPayload {
        session_id_proposal,
        dialler_peer_id,
        capabilities,
        proposed_clearance,
        dialler_epoch_ns,
        dialler_handshake_hash,
    })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
    InvalidClearance(u8),
}

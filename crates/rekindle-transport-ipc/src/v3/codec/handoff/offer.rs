#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffOfferPayload {
    pub handoff_id: uuid::Uuid,
    pub fd_kind: u8,
    pub fd_sealed: bool,
    pub stream_id: u8,
    pub offer_timeout_ms: u32,
    pub payload_size_bytes: u64,
    pub content_hash: [u8; 32],
}

pub fn encode(p: &HandoffOfferPayload) -> Vec<u8> {
    let mut buf = Vec::with_capacity(96);
    buf.extend_from_slice(p.handoff_id.as_bytes());                  // 0..16
    buf.push(p.fd_kind);                                             // 16
    buf.push(u8::from(p.fd_sealed));                                 // 17
    buf.push(p.stream_id);                                           // 18
    buf.push(0);                                                     // 19 reserved
    buf.extend_from_slice(&p.offer_timeout_ms.to_le_bytes());        // 20..24
    buf.extend_from_slice(&p.payload_size_bytes.to_le_bytes());      // 24..32
    buf.extend_from_slice(&p.content_hash);                          // 32..64
    // handoff_mac (64..96) computed by the transport layer at send time
    // using the HandoffKey. The codec produces the MAC input; the
    // caller appends the 32-byte MAC. For roundtrip testing, we pad with zeros.
    buf.extend_from_slice(&[0u8; 32]);                               // 64..96 mac placeholder
    buf
}

pub fn decode(buf: &[u8]) -> Result<HandoffOfferPayload, CodecError> {
    if buf.len() < 96 {
        return Err(CodecError::TooShort { expected: 96, got: buf.len() });
    }
    let handoff_id = uuid::Uuid::from_bytes(buf[0..16].try_into().unwrap());
    let fd_kind = buf[16];
    let fd_sealed = buf[17] != 0;
    let stream_id = buf[18];
    let offer_timeout_ms = u32::from_le_bytes(buf[20..24].try_into().unwrap());
    let payload_size_bytes = u64::from_le_bytes(buf[24..32].try_into().unwrap());
    let mut content_hash = [0u8; 32];
    content_hash.copy_from_slice(&buf[32..64]);
    // handoff_mac at buf[64..96] verified separately by transport layer

    Ok(HandoffOfferPayload { handoff_id, fd_kind, fd_sealed, stream_id, offer_timeout_ms, payload_size_bytes, content_hash })
}

/// Compute the HandoffMAC for an offer payload using the HandoffKey.
/// MAC covers bytes 0..64 (the offer fields before the MAC slot).
pub fn compute_mac(payload: &[u8], handoff_key: &[u8; 32]) -> [u8; 16] {
    let mac_input = if payload.len() >= 64 { &payload[..64] } else { payload };
    let full_mac = blake3::keyed_hash(handoff_key, mac_input);
    let mut truncated = [0u8; 16];
    truncated.copy_from_slice(&full_mac.as_bytes()[..16]);
    truncated
}

/// Verify the HandoffMAC on a raw offer payload against the HandoffKey.
/// Returns true if the MAC at bytes 64..80 matches the computed MAC.
pub fn verify_mac(payload: &[u8], handoff_key: &[u8; 32]) -> bool {
    if payload.len() < 80 {
        return false;
    }
    let expected = compute_mac(payload, handoff_key);
    // Constant-time compare
    let offered = &payload[64..80];
    subtle::ConstantTimeEq::ct_eq(&expected[..], offered).into()
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
}

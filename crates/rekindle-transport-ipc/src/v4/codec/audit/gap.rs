#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditGapPayload {
    pub gap_id: uuid::Uuid,
    pub gap_start_seq: u64,
    pub gap_end_seq: u64,
    pub gap_detected_ns: u64,
    pub expected_chain_link: [u8; 32],
    pub missing_bitmap: Vec<u8>,
}

pub fn encode(p: &AuditGapPayload) -> Vec<u8> {
    let missing_count = p.missing_bitmap.iter().map(|b| b.count_ones()).sum::<u32>();
    let mut buf = Vec::with_capacity(80 + p.missing_bitmap.len());
    buf.extend_from_slice(p.gap_id.as_bytes());                              // 0..16
    buf.extend_from_slice(&p.gap_start_seq.to_le_bytes());                   // 16..24
    buf.extend_from_slice(&p.gap_end_seq.to_le_bytes());                     // 24..32
    buf.extend_from_slice(&p.gap_detected_ns.to_le_bytes());                 // 32..40
    crate::v4::codec::write_u32_len(&mut buf, p.missing_bitmap.len());
    buf.extend_from_slice(&missing_count.to_le_bytes());                     // 44..48
    buf.extend_from_slice(&p.expected_chain_link);                           // 48..80
    buf.extend_from_slice(&p.missing_bitmap);                                // 80..
    buf
}

pub fn decode(buf: &[u8]) -> Result<AuditGapPayload, CodecError> {
    if buf.len() < 80 {
        return Err(CodecError::TooShort { expected: 80, got: buf.len() });
    }
    let gap_id = uuid::Uuid::from_bytes(buf[0..16].try_into().unwrap());
    let gap_start_seq = u64::from_le_bytes(buf[16..24].try_into().unwrap());
    let gap_end_seq = u64::from_le_bytes(buf[24..32].try_into().unwrap());
    let gap_detected_ns = u64::from_le_bytes(buf[32..40].try_into().unwrap());
    let bitmap_len = u32::from_le_bytes(buf[40..44].try_into().unwrap()) as usize;
    // missing_count at 44..48 is informational, not needed for decode
    let mut expected_chain_link = [0u8; 32];
    expected_chain_link.copy_from_slice(&buf[48..80]);

    if buf.len() < 80 + bitmap_len {
        return Err(CodecError::TooShort { expected: 80 + bitmap_len, got: buf.len() });
    }
    let missing_bitmap = buf[80..80 + bitmap_len].to_vec();

    Ok(AuditGapPayload { gap_id, gap_start_seq, gap_end_seq, gap_detected_ns, expected_chain_link, missing_bitmap })
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
}

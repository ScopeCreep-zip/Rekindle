//! CHANNEL_ERROR codec.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorPayload {
    pub error_code: u16,
    pub message: String,
}

pub fn decode(data: &[u8]) -> Result<ErrorPayload, CodecError> {
    if data.len() < 8 {
        return Err(CodecError::TooShort { expected: 8, actual: data.len() });
    }
    let error_code = u16::from_le_bytes([data[0], data[1]]);
    let message_len = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
    if data.len() < 8 + message_len {
        return Err(CodecError::TruncatedMessage);
    }
    let message = String::from_utf8_lossy(&data[8..8 + message_len]).into_owned();
    Ok(ErrorPayload { error_code, message })
}

pub fn encode(p: &ErrorPayload) -> Vec<u8> {
    let msg_bytes = p.message.as_bytes();
    let mut buf = Vec::with_capacity(8 + msg_bytes.len());
    buf.extend_from_slice(&p.error_code.to_le_bytes());
    buf.extend_from_slice(&[0, 0]); // reserved
    let msg_len = u32::try_from(msg_bytes.len()).expect("error message exceeds u32");
    buf.extend_from_slice(&msg_len.to_le_bytes());
    buf.extend_from_slice(msg_bytes);
    buf
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, actual: usize },
    TruncatedMessage,
}

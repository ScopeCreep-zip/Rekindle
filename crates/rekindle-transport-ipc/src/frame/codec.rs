//! Postcard serialization and length-prefixed wire framing.
//!
//! Wire format: `[4-byte LE u32 length][postcard payload]`
//!
//! Little-endian matches x86-64/ARM64 native order and postcard's wire
//! format. Fixed-width permits zero-copy header read. 4-byte overhead
//! is <4% at 100+ byte average frame.
//!
//! Length is validated BEFORE allocation (reject-before-allocate).
//! A malicious `0xFFFFFFFF` prefix is rejected without 4 GiB alloc.

use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::{IpcError, IpcResult};

/// Maximum frame payload size: 16 MiB. Prevents OOM from malformed prefixes.
pub const MAX_FRAME_SIZE: u32 = 16 * 1024 * 1024;

/// Serialize a value to postcard bytes.
///
/// Symmetric with [`decode_frame`]: `decode_frame(&encode_frame(&v)) == v`.
pub fn encode_frame<T: Serialize>(value: &T) -> IpcResult<Vec<u8>> {
    postcard::to_allocvec(value).map_err(|e| IpcError::SerializationFailed {
        reason: e.to_string(),
    })
}

/// Deserialize a value from postcard bytes.
///
/// Symmetric with [`encode_frame`]. Never panics on malformed input.
pub fn decode_frame<T: DeserializeOwned>(payload: &[u8]) -> IpcResult<T> {
    postcard::from_bytes(payload).map_err(|e| IpcError::DeserializationFailed {
        reason: e.to_string(),
    })
}

/// Read a single length-prefixed frame from an async reader.
///
/// Validates length against `max` BEFORE allocating.
/// Returns `Err(ConnectionClosed)` on clean EOF.
pub async fn read_frame<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut R,
    max: u32,
) -> IpcResult<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(IpcError::ConnectionClosed);
        }
        Err(e) => return Err(IpcError::Io(e)),
    }

    let len = u32::from_le_bytes(len_buf);
    if len > max {
        return Err(IpcError::FrameTooLarge { size: len, max });
    }

    let mut payload = vec![0u8; len as usize];
    reader.read_exact(&mut payload).await?;
    Ok(payload)
}

/// Write a single length-prefixed frame to an async writer.
///
/// Writes the 4-byte LE length prefix and payload. Does NOT flush —
/// the caller is responsible for flushing after a batch of frames.
pub async fn write_frame<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    payload: &[u8],
) -> IpcResult<()> {
    let len = u32::try_from(payload.len()).map_err(|_| IpcError::FrameTooLarge {
        size: u32::MAX,
        max: MAX_FRAME_SIZE,
    })?;
    writer.write_all(&len.to_le_bytes()).await?;
    writer.write_all(payload).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip() {
        let value: (u32, String) = (42, "hello".into());
        let bytes = encode_frame(&value).unwrap();
        let decoded: (u32, String) = decode_frame(&bytes).unwrap();
        assert_eq!(decoded, value);
    }

    #[test]
    fn decode_malformed_returns_err() {
        let garbage = vec![0xFF, 0xFE, 0xFD, 0xFC, 0xFB];
        let result: Result<(u32, String), _> = decode_frame(&garbage);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn read_write_frame_roundtrip() {
        let payload = b"hello postcard";
        let mut buf = Vec::new();
        write_frame(&mut buf, payload).await.unwrap();
        let mut cursor = &buf[..];
        let decoded = read_frame(&mut cursor, MAX_FRAME_SIZE).await.unwrap();
        assert_eq!(decoded, payload);
    }

    #[tokio::test]
    async fn oversized_frame_rejected() {
        let oversized_len: u32 = MAX_FRAME_SIZE + 1;
        let mut buf = Vec::new();
        buf.extend_from_slice(&oversized_len.to_le_bytes());
        buf.extend(std::iter::repeat_n(0u8, 64));
        let mut cursor = &buf[..];
        let result = read_frame(&mut cursor, MAX_FRAME_SIZE).await;
        assert!(matches!(result, Err(IpcError::FrameTooLarge { .. })));
    }

    #[tokio::test]
    async fn eof_returns_connection_closed() {
        let buf: &[u8] = &[];
        let mut cursor = buf;
        let result = read_frame(&mut cursor, MAX_FRAME_SIZE).await;
        assert!(matches!(result, Err(IpcError::ConnectionClosed)));
    }
}

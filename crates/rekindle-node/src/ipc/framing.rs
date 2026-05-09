//! Postcard serialization and length-prefixed wire framing.
//!
//! Two clean layers:
//! - **Serialization**: `encode_frame` / `decode_frame` convert between typed
//!   values and postcard byte payloads. Symmetric: encode produces what decode consumes.
//! - **Wire I/O**: `write_frame` / `read_frame` add/strip a 4-byte big-endian
//!   length prefix for socket transport.
//!
//! Wire format on the socket: `[4-byte BE length][postcard payload]`
//!
//! Internal routing (bus dispatch, mpsc channels) carries raw postcard payloads
//! without the length prefix — enabling zero-copy forwarding.
//!
//! [RC-2] No `.unwrap()` on any decode of untrusted data.
//! [RC-3] Frame length validated before allocation (OOM protection).


use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::error::{IpcError, Result};

/// Maximum frame payload size: 16 MiB. Prevents OOM from malformed length prefixes.
pub const MAX_FRAME_SIZE: u32 = 16 * 1024 * 1024;

/// Serialize a value to postcard bytes.
///
/// Symmetric with [`decode_frame`]: `decode_frame(encode_frame(v)) == v`.
pub fn encode_frame<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    postcard::to_allocvec(value).map_err(|e| IpcError::SerializationFailed {
        reason: e.to_string(),
    })
}

/// Deserialize a value from postcard bytes.
///
/// Symmetric with [`encode_frame`]. Never panics on malformed input — returns
/// `Err(DeserializationFailed)` instead. [RC-2]
pub fn decode_frame<T: DeserializeOwned>(payload: &[u8]) -> Result<T> {
    postcard::from_bytes(payload).map_err(|e| IpcError::DeserializationFailed {
        reason: e.to_string(),
    })
}

/// Read a single length-prefixed frame from an async reader.
///
/// Rejects frames exceeding [`MAX_FRAME_SIZE`] before allocating. [RC-3]
///
/// Returns `Err(ConnectionClosed)` on clean EOF (0 bytes read for length).
pub async fn read_frame<R: tokio::io::AsyncRead + Unpin>(reader: &mut R) -> Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(IpcError::ConnectionClosed);
        }
        Err(e) => return Err(IpcError::Io(e)),
    }

    let len = u32::from_be_bytes(len_buf);
    tracing::trace!(
        frame_len = len,
        len_hex = %format!("{:02x}{:02x}{:02x}{:02x}", len_buf[0], len_buf[1], len_buf[2], len_buf[3]),
        "read_frame: length prefix read"
    );
    if len > MAX_FRAME_SIZE {
        return Err(IpcError::FrameTooLarge {
            size: len,
            max: MAX_FRAME_SIZE,
        });
    }

    // [RC-3] len is validated ≤ MAX_FRAME_SIZE (16 MiB) before allocation.
    // u32 → usize conversion is safe on all 32-bit+ platforms.
    let mut payload = vec![0u8; len as usize];
    reader.read_exact(&mut payload).await?;
    Ok(payload)
}

/// Write a single length-prefixed frame to an async writer.
pub async fn write_frame<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    payload: &[u8],
) -> Result<()> {
    // [RC-3] TryFrom prevents silent truncation on hypothetical >4GiB payloads.
    let len = u32::try_from(payload.len()).map_err(|_| IpcError::FrameTooLarge {
        size: u32::MAX,
        max: MAX_FRAME_SIZE,
    })?;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(payload).await?;
    writer.flush().await?;
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
    fn decode_malformed_returns_err_not_panic() {
        // [RC-2] Garbage bytes must not panic.
        let garbage = vec![0xFF, 0xFE, 0xFD, 0xFC, 0xFB];
        let result: std::result::Result<(u32, String), _> = decode_frame(&garbage);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn read_write_frame_roundtrip() {
        let payload = b"hello postcard";
        let mut buf = Vec::new();
        write_frame(&mut buf, payload).await.unwrap();
        let mut cursor = &buf[..];
        let decoded = read_frame(&mut cursor).await.unwrap();
        assert_eq!(decoded, payload);
    }

    #[tokio::test]
    async fn oversized_frame_rejected() {
        // [RC-3] Frame exceeding MAX_FRAME_SIZE rejected before allocation.
        let oversized_len: u32 = MAX_FRAME_SIZE + 1;
        let mut buf = Vec::new();
        buf.extend_from_slice(&oversized_len.to_be_bytes());
        buf.extend(std::iter::repeat_n(0u8, 64));
        let mut cursor = &buf[..];
        let result = read_frame(&mut cursor).await;
        assert!(matches!(result, Err(IpcError::FrameTooLarge { .. })));
    }

    #[tokio::test]
    async fn zero_length_frame_roundtrips() {
        let payload: &[u8] = &[];
        let mut buf = Vec::new();
        write_frame(&mut buf, payload).await.unwrap();
        let mut cursor = &buf[..];
        let decoded = read_frame(&mut cursor).await.unwrap();
        assert!(decoded.is_empty());
    }

    #[tokio::test]
    async fn eof_returns_connection_closed() {
        let buf: &[u8] = &[];
        let mut cursor = buf;
        let result = read_frame(&mut cursor).await;
        assert!(matches!(result, Err(IpcError::ConnectionClosed)));
    }
}

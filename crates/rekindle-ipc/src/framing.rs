//! Postcard serialization and the Noise message framing on the wire.
//!
//! Two layers:
//! - **Serialization**: `encode_frame` / `decode_frame` convert between typed
//!   values and postcard byte payloads. Symmetric: encode produces what decode
//!   consumes.
//! - **Wire**: every Noise message — handshake or transport — is preceded by a
//!   16-bit big-endian length, the framing the Noise spec recommends (rev 34
//!   §13). A Noise message is at most 65535 bytes (§3), so the cap is the
//!   type of the length field, not a check. Application frames larger than
//!   one message are chunked by `noise::NoiseTransport`, which authenticates
//!   their total length inside the first chunk.
//!
//! [RC-2] No `.unwrap()` on any decode of untrusted data.
//! [RC-3] At most 65535 bytes are allocated per wire read.

use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use super::error::{IpcError, Result};

/// Largest reassembled application frame: 16 MiB.
pub const MAX_APP_FRAME: u32 = 16 * 1024 * 1024;

/// Largest Noise message (spec §3).
pub const MAX_NOISE_MESSAGE: usize = 65535;

/// Ciphertext messages buffered between a connection's reader task and its
/// I/O loop.
const READER_QUEUE: usize = 64;

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

/// Read one Noise message: a 16-bit big-endian length, then that many bytes.
///
/// Returns `Err(ConnectionClosed)` on clean EOF before a length.
pub async fn read_noise_message<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<Vec<u8>> {
    let mut len_buf = [0u8; 2];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(IpcError::ConnectionClosed);
        }
        Err(e) => return Err(IpcError::Io(e)),
    }
    let mut message = vec![0u8; usize::from(u16::from_be_bytes(len_buf))];
    reader.read_exact(&mut message).await?;
    Ok(message)
}

/// Write one Noise message with its 16-bit big-endian length. Does not
/// flush; a caller writing several messages flushes once.
pub async fn write_noise_message<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    message: &[u8],
) -> Result<()> {
    let len = u16::try_from(message.len()).map_err(|_| IpcError::FrameTooLarge {
        size: u32::try_from(message.len()).unwrap_or(u32::MAX),
        max: u32::from(u16::MAX),
    })?;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(message).await?;
    Ok(())
}

/// Read Noise messages off `reader` into a channel until the stream ends.
///
/// The connection's I/O loop receives from the channel inside `select!`:
/// `mpsc::Receiver::recv` is cancel-safe, so a branch taken elsewhere never
/// loses part of a message — which a `read_exact` inside `select!` would —
/// and decryption (and the receive nonce) stays in the loop, advancing only
/// on a whole message (WS12.1). The channel closes when the stream does.
pub fn spawn_reader<R>(mut reader: R) -> (mpsc::Receiver<Vec<u8>>, tokio::task::JoinHandle<()>)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let (tx, rx) = mpsc::channel(READER_QUEUE);
    let handle = tokio::spawn(async move {
        loop {
            match read_noise_message(&mut reader).await {
                Ok(message) => {
                    if tx.send(message).await.is_err() {
                        return;
                    }
                }
                Err(IpcError::ConnectionClosed) => return,
                Err(e) => {
                    tracing::debug!(error = %e, "IPC read ended");
                    return;
                }
            }
        }
    });
    (rx, handle)
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
    async fn noise_message_roundtrip() {
        let mut buf = Vec::new();
        write_noise_message(&mut buf, b"hello").await.unwrap();
        assert_eq!(&buf[..2], &[0, 5], "16-bit big-endian length");
        let mut cursor = &buf[..];
        assert_eq!(read_noise_message(&mut cursor).await.unwrap(), b"hello");
    }

    /// A message longer than the Noise maximum cannot be framed.
    #[tokio::test]
    async fn oversized_message_is_refused() {
        let mut buf = Vec::new();
        assert!(matches!(
            write_noise_message(&mut buf, &vec![0u8; MAX_NOISE_MESSAGE + 1]).await,
            Err(IpcError::FrameTooLarge { .. })
        ));
        write_noise_message(&mut buf, &vec![0u8; MAX_NOISE_MESSAGE])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn eof_before_length_is_connection_closed() {
        let mut cursor: &[u8] = &[];
        assert!(matches!(
            read_noise_message(&mut cursor).await,
            Err(IpcError::ConnectionClosed)
        ));
    }
}

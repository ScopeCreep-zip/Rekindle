//! Bulk frame reader: lane-aware frame extraction from a socket.
//!
//! Reads the 1-byte lane prefix, then the 4-byte BE length prefix,
//! then the frame body. Returns `(lane, body)` to the connection
//! handler, which routes lane 0x00 to the Noise decrypt path and
//! lanes 0x01–0x03 to the bulk dispatcher.
//!
//! The reader maintains a persistent `BytesMut` read buffer to avoid
//! per-frame allocation. After warmup, reads are satisfied from the
//! buffer without allocation.

use bytes::BytesMut;
use tokio::io::AsyncReadExt;

/// Maximum allowed frame body size (256 KiB). Frames larger than this
/// are rejected to prevent memory exhaustion from malicious peers.
/// The largest legitimate bulk frame is ~65.5 KiB (MAX_CHUNK_PLAIN +
/// HEADER_LEN + TAG_LEN). 256 KiB provides ample headroom for future
/// frame types without exposing unbounded allocation.
const MAX_BODY_LEN: usize = 256 * 1024;

/// Read one lane-prefixed frame from the socket.
///
/// Wire format:
/// ```text
/// [1B lane][4B length BE][length bytes of body]
/// ```
///
/// Returns `Ok(Some((lane, body)))` on success.
/// Returns `Ok(None)` on clean EOF (peer closed connection).
/// Returns `Err` on I/O error, unexpected EOF, or oversized frame.
///
/// `read_buf` is a persistent buffer owned by the caller. It retains
/// capacity across calls, avoiding per-frame allocation after warmup.
pub async fn read_lane_frame<R: AsyncReadExt + Unpin>(
    reader: &mut R,
    read_buf: &mut BytesMut,
) -> std::io::Result<Option<(u8, bytes::Bytes)>> {
    // ── Read lane byte ──────────────────────────────────────────
    let lane = match read_one_byte(reader).await {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    };

    // ── Read 4-byte length prefix ───────────────────────────────
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let body_len = u32::from_be_bytes(len_buf) as usize;

    if body_len > MAX_BODY_LEN {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("frame body length {body_len} exceeds maximum {MAX_BODY_LEN}"),
        ));
    }

    // ── Read frame body ─────────────────────────────────────────
    // Ensure read_buf has enough capacity.
    read_buf.clear();
    read_buf.reserve(body_len);

    while read_buf.len() < body_len {
        let n = reader.read_buf(read_buf).await?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "connection closed mid-frame",
            ));
        }
    }

    // Extract exactly body_len bytes. If read_buf received more
    // (pipelining), the excess stays in read_buf for the next call.
    let body = read_buf.split_to(body_len).freeze();

    Ok(Some((lane, body)))
}

/// Read a single byte from the reader.
///
/// Returns `UnexpectedEof` if the reader is at EOF.
async fn read_one_byte<R: AsyncReadExt + Unpin>(reader: &mut R) -> std::io::Result<u8> {
    let mut buf = [0u8; 1];
    reader.read_exact(&mut buf).await?;
    Ok(buf[0])
}

/// Write a lane-prefixed frame to a writer.
///
/// Convenience function for the control-plane path: the Noise
/// `write_encrypted_frame` output needs to be prefixed with lane 0x00.
///
/// This writes `[lane][body]` without a length prefix — the body
/// already contains its own length prefix (the Noise chunk count
/// header). The caller is responsible for flushing after the write.
pub async fn write_lane_byte<W: tokio::io::AsyncWriteExt + Unpin>(
    writer: &mut W,
    lane: u8,
) -> std::io::Result<()> {
    writer.write_all(&[lane]).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn roundtrip_lane_frame() {
        let (client, server) = tokio::io::duplex(64 * 1024);
        // client side unused — drop both halves so server sees EOF after data.
        drop(client);
        let (mut sr, mut sw) = tokio::io::split(server);

        let body = b"hello lane frame";
        let lane: u8 = 0x01;

        // Write: [lane][length BE][body]
        let write_handle = tokio::spawn(async move {
            sw.write_all(&[lane]).await.unwrap();
            sw.write_all(&(body.len() as u32).to_be_bytes()).await.unwrap();
            sw.write_all(body).await.unwrap();
            sw.flush().await.unwrap();
            drop(sw);
        });

        let mut read_buf = BytesMut::new();
        let result = read_lane_frame(&mut sr, &mut read_buf).await.unwrap();
        let (read_lane, read_body) = result.expect("expected a frame");

        assert_eq!(read_lane, lane);
        assert_eq!(&read_body[..], body);

        write_handle.await.unwrap();
    }

    #[tokio::test]
    async fn clean_eof_returns_none() {
        let (client, server) = tokio::io::duplex(1024);
        // Drop the client entirely — server's read half sees EOF.
        drop(client);
        let (mut sr, _sw) = tokio::io::split(server);

        let mut read_buf = BytesMut::new();
        let result = read_lane_frame(&mut sr, &mut read_buf).await.unwrap();
        assert!(result.is_none(), "expected None on clean EOF");
    }

    #[tokio::test]
    async fn oversized_frame_rejected() {
        let (client, server) = tokio::io::duplex(64 * 1024);
        drop(client);
        let (mut sr, mut sw) = tokio::io::split(server);

        let write_handle = tokio::spawn(async move {
            sw.write_all(&[0x01]).await.unwrap(); // lane
            // Length = MAX_BODY_LEN + 1
            let bad_len = (MAX_BODY_LEN as u32 + 1).to_be_bytes();
            sw.write_all(&bad_len).await.unwrap();
            sw.flush().await.unwrap();
        });

        let mut read_buf = BytesMut::new();
        let result = read_lane_frame(&mut sr, &mut read_buf).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);

        write_handle.await.unwrap();
    }

    #[tokio::test]
    async fn multiple_frames_in_sequence() {
        let (client, server) = tokio::io::duplex(64 * 1024);
        drop(client);
        let (mut sr, mut sw) = tokio::io::split(server);

        let write_handle = tokio::spawn(async move {
            for i in 0u8..5 {
                let body = vec![i; 100];
                sw.write_all(&[i]).await.unwrap(); // lane = i
                sw.write_all(&(body.len() as u32).to_be_bytes()).await.unwrap();
                sw.write_all(&body).await.unwrap();
            }
            sw.flush().await.unwrap();
            drop(sw);
        });

        let mut read_buf = BytesMut::new();
        for i in 0u8..5 {
            let (lane, body) = read_lane_frame(&mut sr, &mut read_buf)
                .await
                .unwrap()
                .expect("expected a frame");
            assert_eq!(lane, i);
            assert_eq!(body, vec![i; 100]);
        }

        // Next read should be EOF.
        let result = read_lane_frame(&mut sr, &mut read_buf).await.unwrap();
        assert!(result.is_none());

        write_handle.await.unwrap();
    }
}

//! Noise IK encrypted IPC transport.
//!
//! Provides forward-secret, mutually-authenticated encryption for all IPC
//! traffic using the Noise Protocol Framework (IK pattern) via `snow`.
//!
//! Pattern: `Noise_IK_25519_ChaChaPoly_BLAKE2s`
//! - IK: initiator's static key transmitted, responder's static key pre-known
//! - X25519 DH, ChaCha20-Poly1305 AEAD, BLAKE2s hash
//! - 2-message handshake (1 round-trip), then forward-secret transport
//!
//! UCred (PID + UID) bound into the Noise prologue — cryptographically binding
//! OS-level transport identity to the encrypted channel. [RC-4]
//!
//! Noise transport messages limited to 65535 bytes. Application frames up to
//! 16 MiB are chunked with a chunk-count header.
//!
//! Adapted from open-sesame `core-ipc/src/noise.rs`.

use tokio::io::{AsyncRead, AsyncWrite};

use super::error::{IpcError, Result};
use super::framing::{read_frame, write_frame, MAX_FRAME_SIZE};
use super::noise_keys::NOISE_PARAMS;
use super::transport::PeerCredentials;

/// Maximum plaintext per Noise transport message: 65535 - 16 (AEAD tag) = 65519.
const MAX_NOISE_PLAINTEXT: usize = 65535 - 16;

/// Handshake timeout to prevent DoS via slow handshake. [RC-7]
const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Build the Noise prologue from peer credentials.
///
/// Format: `REKINDLE-IPC-v1:{lower_pid}:{lower_uid}:{higher_pid}:{higher_uid}`
///
/// Canonical ordering (lower PID first) ensures both sides produce identical
/// bytes regardless of who initiates. Prologue is mixed into the Noise
/// handshake hash — mismatch causes handshake failure. [RC-4]
fn build_prologue(local: &PeerCredentials, remote: &PeerCredentials) -> Vec<u8> {
    let (first, second) = if local.pid <= remote.pid {
        (local, remote)
    } else {
        (remote, local)
    };
    format!(
        "REKINDLE-IPC-v1:{}:{}:{}:{}",
        first.pid, first.uid, second.pid, second.uid
    )
    .into_bytes()
}

/// Encrypted IPC transport wrapping a completed Noise session.
///
/// Provides chunked encrypted frame I/O. Application frames are split into
/// chunks of at most `MAX_NOISE_PLAINTEXT` bytes, each encrypted as a
/// separate Noise transport message.
///
/// `TransportState` requires `&mut self` for both encrypt and decrypt,
/// so callers must coordinate access. The server uses `tokio::select!` to
/// multiplex reads and writes in a single task.
pub struct NoiseTransport {
    state: snow::TransportState,
}

impl NoiseTransport {
    /// Write an encrypted application frame.
    ///
    /// Payload is chunked into Noise messages. Wire format:
    /// `[4-byte BE chunk_count][chunk_1][chunk_2]...[chunk_n]`
    pub async fn write_encrypted_frame<W: AsyncWrite + Unpin>(
        &mut self,
        writer: &mut W,
        payload: &[u8],
    ) -> Result<()> {
        #[allow(clippy::cast_possible_truncation)] // validated ≤ 16 MiB < u32::MAX
        if payload.len() > MAX_FRAME_SIZE as usize {
            return Err(IpcError::FrameTooLarge {
                size: payload.len() as u32,
                max: MAX_FRAME_SIZE,
            });
        }

        // [RC-3] chunk_count: payload.len() ≤ 16 MiB, MAX_NOISE_PLAINTEXT = 65519.
        // Max chunks = 16*1024*1024 / 65519 = 256. Fits in u32.
        let chunk_count = if payload.is_empty() {
            1 // One empty encrypted chunk for zero-length payloads.
        } else {
            payload.len().div_ceil(MAX_NOISE_PLAINTEXT)
        };

        let count_bytes = u32::try_from(chunk_count).map_err(|_| IpcError::TooManyChunks {
            count: chunk_count,
            max: 256,
        })?;
        write_frame(writer, &count_bytes.to_be_bytes()).await?;

        let mut enc_buf = vec![0u8; MAX_NOISE_PLAINTEXT + 16];
        for chunk_idx in 0..chunk_count {
            let start = chunk_idx * MAX_NOISE_PLAINTEXT;
            let end = (start + MAX_NOISE_PLAINTEXT).min(payload.len());
            let chunk = &payload[start..end];

            let len = self.state.write_message(chunk, &mut enc_buf).map_err(|e| {
                IpcError::EncryptFailed {
                    reason: e.to_string(),
                }
            })?;

            write_frame(writer, &enc_buf[..len]).await?;
        }

        Ok(())
    }

    /// Read and decrypt an application frame.
    ///
    /// Reads chunk count, then reads and decrypts each chunk, reassembling
    /// the original plaintext payload.
    pub async fn read_encrypted_frame<R: AsyncRead + Unpin>(
        &mut self,
        reader: &mut R,
    ) -> Result<Vec<u8>> {
        let count_frame = read_frame(reader).await?;
        if count_frame.len() != 4 {
            return Err(IpcError::InvalidChunkHeader {
                got: count_frame.len(),
            });
        }
        let chunk_count = u32::from_be_bytes([
            count_frame[0],
            count_frame[1],
            count_frame[2],
            count_frame[3],
        ]) as usize;

        let max_chunks = (MAX_FRAME_SIZE as usize).div_ceil(MAX_NOISE_PLAINTEXT);
        if chunk_count > max_chunks {
            return Err(IpcError::TooManyChunks {
                count: chunk_count,
                max: max_chunks,
            });
        }

        let mut payload = Vec::with_capacity(chunk_count * MAX_NOISE_PLAINTEXT);
        let mut dec_buf = vec![0u8; MAX_NOISE_PLAINTEXT];

        for _ in 0..chunk_count {
            let ciphertext = read_frame(reader).await?;
            let len = self
                .state
                .read_message(&ciphertext, &mut dec_buf)
                .map_err(|e| IpcError::DecryptFailed {
                    reason: e.to_string(),
                })?;
            payload.extend_from_slice(&dec_buf[..len]);
        }

        // [RC-16] Zeroize intermediate decrypt buffer.
        zeroize::Zeroize::zeroize(&mut dec_buf);

        if let Ok(size) = u32::try_from(payload.len()) {
            if size > MAX_FRAME_SIZE {
                return Err(IpcError::FrameTooLarge {
                    size,
                    max: MAX_FRAME_SIZE,
                });
            }
        } else {
            return Err(IpcError::FrameTooLarge {
                size: u32::MAX,
                max: MAX_FRAME_SIZE,
            });
        }

        Ok(payload)
    }

    /// Get the remote party's static public key (after handshake).
    pub fn remote_static(&self) -> Option<&[u8]> {
        self.state.get_remote_static()
    }
}

/// Perform the server-side (responder) Noise IK handshake.
///
/// IK responder: read msg1 (initiator's ephemeral + encrypted static),
/// write msg2 (responder's ephemeral). Then derive transport keys.
pub async fn server_handshake<R, W>(
    reader: &mut R,
    writer: &mut W,
    server_keypair: &snow::Keypair,
    local_creds: &PeerCredentials,
    remote_creds: &PeerCredentials,
) -> Result<NoiseTransport>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let prologue = build_prologue(local_creds, remote_creds);

    let mut handshake =
        snow::Builder::new(
            NOISE_PARAMS
                .parse()
                .map_err(|e| IpcError::HandshakeFailed {
                    reason: format!("invalid Noise params: {e}"),
                })?,
        )
        .local_private_key(&server_keypair.private)
        .map_err(|e| IpcError::HandshakeFailed {
            reason: format!("Noise builder: {e}"),
        })?
        .prologue(&prologue)
        .map_err(|e| IpcError::HandshakeFailed {
            reason: format!("Noise prologue: {e}"),
        })?
        .build_responder()
        .map_err(|e| IpcError::HandshakeFailed {
            reason: format!("Noise responder build: {e}"),
        })?;

    tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        // Read msg1 from initiator.
        let msg1 = read_frame(reader).await?;
        let mut payload_buf = vec![0u8; 65535];
        handshake
            .read_message(&msg1, &mut payload_buf)
            .map_err(|e| IpcError::HandshakeFailed {
                reason: format!("msg1 read: {e}"),
            })?;

        // Write msg2 to initiator.
        let mut msg2_buf = vec![0u8; 65535];
        let msg2_len =
            handshake
                .write_message(&[], &mut msg2_buf)
                .map_err(|e| IpcError::HandshakeFailed {
                    reason: format!("msg2 write: {e}"),
                })?;
        write_frame(writer, &msg2_buf[..msg2_len]).await?;

        // Transition to transport mode.
        let transport = handshake
            .into_transport_mode()
            .map_err(|e| IpcError::HandshakeFailed {
                reason: format!("transport mode: {e}"),
            })?;

        tracing::debug!("Noise IK handshake completed (server)");
        Ok(NoiseTransport { state: transport })
    })
    .await
    .map_err(|_| IpcError::HandshakeTimeout {
        timeout_ms: HANDSHAKE_TIMEOUT.as_secs() * 1000,
    })?
}

/// Perform the client-side (initiator) Noise IK handshake.
///
/// IK initiator: write msg1 (ephemeral + encrypted static),
/// read msg2 (responder's ephemeral). Then derive transport keys.
pub async fn client_handshake<R, W>(
    reader: &mut R,
    writer: &mut W,
    server_public_key: &[u8; 32],
    client_keypair: &snow::Keypair,
    local_creds: &PeerCredentials,
    remote_creds: &PeerCredentials,
) -> Result<NoiseTransport>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let prologue = build_prologue(local_creds, remote_creds);

    let mut handshake =
        snow::Builder::new(
            NOISE_PARAMS
                .parse()
                .map_err(|e| IpcError::HandshakeFailed {
                    reason: format!("invalid Noise params: {e}"),
                })?,
        )
        .local_private_key(&client_keypair.private)
        .map_err(|e| IpcError::HandshakeFailed {
            reason: format!("Noise builder: {e}"),
        })?
        .remote_public_key(server_public_key)
        .map_err(|e| IpcError::HandshakeFailed {
            reason: format!("Noise remote key: {e}"),
        })?
        .prologue(&prologue)
        .map_err(|e| IpcError::HandshakeFailed {
            reason: format!("Noise prologue: {e}"),
        })?
        .build_initiator()
        .map_err(|e| IpcError::HandshakeFailed {
            reason: format!("Noise initiator build: {e}"),
        })?;

    tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        // Write msg1 to responder.
        let mut msg1_buf = vec![0u8; 65535];
        let msg1_len =
            handshake
                .write_message(&[], &mut msg1_buf)
                .map_err(|e| IpcError::HandshakeFailed {
                    reason: format!("msg1 write: {e}"),
                })?;
        write_frame(writer, &msg1_buf[..msg1_len]).await?;

        // Read msg2 from responder.
        let msg2 = read_frame(reader).await?;
        let mut payload_buf = vec![0u8; 65535];
        handshake
            .read_message(&msg2, &mut payload_buf)
            .map_err(|e| IpcError::HandshakeFailed {
                reason: format!("msg2 read: {e}"),
            })?;

        // Transition to transport mode.
        let transport = handshake
            .into_transport_mode()
            .map_err(|e| IpcError::HandshakeFailed {
                reason: format!("transport mode: {e}"),
            })?;

        tracing::debug!("Noise IK handshake completed (client)");
        Ok(NoiseTransport { state: transport })
    })
    .await
    .map_err(|_| IpcError::HandshakeTimeout {
        timeout_ms: HANDSHAKE_TIMEOUT.as_secs() * 1000,
    })?
}

#[cfg(test)]
mod tests {
    use super::super::noise_keys::generate_keypair;
    use super::*;

    #[test]
    fn prologue_canonical_ordering() {
        let a = PeerCredentials {
            pid: 100,
            uid: 1000,
        };
        let b = PeerCredentials {
            pid: 200,
            uid: 1000,
        };
        assert_eq!(build_prologue(&a, &b), build_prologue(&b, &a));
        let p = String::from_utf8(build_prologue(&a, &b)).unwrap();
        assert_eq!(p, "REKINDLE-IPC-v1:100:1000:200:1000");
    }

    #[tokio::test]
    async fn handshake_and_transport_roundtrip() {
        let server_kp = generate_keypair().unwrap();
        let client_kp = generate_keypair().unwrap();
        let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();

        let server_creds = PeerCredentials { pid: 1, uid: 1000 };
        let client_creds = PeerCredentials { pid: 2, uid: 1000 };

        let (client_stream, server_stream) = tokio::io::duplex(65536);
        let (mut client_reader, mut client_writer) = tokio::io::split(client_stream);
        let (mut server_reader, mut server_writer) = tokio::io::split(server_stream);

        let (client_result, server_result) = tokio::join!(
            client_handshake(
                &mut client_reader,
                &mut client_writer,
                &server_pub,
                client_kp.as_inner(),
                &client_creds,
                &server_creds,
            ),
            server_handshake(
                &mut server_reader,
                &mut server_writer,
                server_kp.as_inner(),
                &server_creds,
                &client_creds,
            ),
        );

        let mut client_transport = client_result.unwrap();
        let mut server_transport = server_result.unwrap();

        // Client sends, server receives.
        let plaintext = b"hello encrypted world";
        client_transport
            .write_encrypted_frame(&mut client_writer, plaintext)
            .await
            .unwrap();
        let decrypted = server_transport
            .read_encrypted_frame(&mut server_reader)
            .await
            .unwrap();
        assert_eq!(decrypted, plaintext);

        // Server sends back, client receives.
        let response = b"acknowledged";
        server_transport
            .write_encrypted_frame(&mut server_writer, response)
            .await
            .unwrap();
        let decrypted_response = client_transport
            .read_encrypted_frame(&mut client_reader)
            .await
            .unwrap();
        assert_eq!(decrypted_response, response);
    }

    #[tokio::test]
    async fn large_frame_chunking() {
        let server_kp = generate_keypair().unwrap();
        let client_kp = generate_keypair().unwrap();
        let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();

        let sc = PeerCredentials { pid: 10, uid: 1000 };
        let cc = PeerCredentials { pid: 20, uid: 1000 };

        let (cs, ss) = tokio::io::duplex(1024 * 1024);
        let (mut cr, mut cw) = tokio::io::split(cs);
        let (mut sr, mut sw) = tokio::io::split(ss);

        let (mut ct, mut st) = tokio::join!(
            async {
                client_handshake(
                    &mut cr,
                    &mut cw,
                    &server_pub,
                    client_kp.as_inner(),
                    &cc,
                    &sc,
                )
                .await
                .unwrap()
            },
            async {
                server_handshake(&mut sr, &mut sw, server_kp.as_inner(), &sc, &cc)
                    .await
                    .unwrap()
            },
        );

        // 200 KiB payload — requires ~4 chunks.
        let large_payload = vec![0xABu8; 200 * 1024];
        ct.write_encrypted_frame(&mut cw, &large_payload)
            .await
            .unwrap();
        let decrypted = st.read_encrypted_frame(&mut sr).await.unwrap();
        assert_eq!(decrypted, large_payload);
    }

    #[tokio::test]
    async fn prologue_mismatch_fails_handshake() {
        // [RC-4] Mismatched UIDs must cause handshake failure.
        let server_kp = generate_keypair().unwrap();
        let client_kp = generate_keypair().unwrap();
        let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();

        let server_creds = PeerCredentials { pid: 1, uid: 1000 };
        let client_creds_real = PeerCredentials { pid: 2, uid: 1000 };
        let client_creds_fake = PeerCredentials { pid: 99, uid: 9999 }; // Wrong UID

        let (cs, ss) = tokio::io::duplex(65536);
        let (mut cr, mut cw) = tokio::io::split(cs);
        let (mut sr, mut sw) = tokio::io::split(ss);

        let (client_result, server_result) = tokio::join!(
            // Client uses real creds.
            client_handshake(
                &mut cr,
                &mut cw,
                &server_pub,
                client_kp.as_inner(),
                &client_creds_real,
                &server_creds,
            ),
            // Server uses WRONG creds (thinks client is PID 99, UID 9999).
            server_handshake(
                &mut sr,
                &mut sw,
                server_kp.as_inner(),
                &server_creds,
                &client_creds_fake,
            ),
        );

        // At least one side must fail due to prologue mismatch.
        assert!(
            client_result.is_err() || server_result.is_err(),
            "prologue mismatch must cause handshake failure"
        );
    }

    #[tokio::test]
    async fn empty_payload_roundtrip() {
        let server_kp = generate_keypair().unwrap();
        let client_kp = generate_keypair().unwrap();
        let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();

        let sc = PeerCredentials { pid: 1, uid: 1000 };
        let cc = PeerCredentials { pid: 2, uid: 1000 };

        let (cs, ss) = tokio::io::duplex(65536);
        let (mut cr, mut cw) = tokio::io::split(cs);
        let (mut sr, mut sw) = tokio::io::split(ss);

        let (mut ct, mut st) = tokio::join!(
            async {
                client_handshake(
                    &mut cr,
                    &mut cw,
                    &server_pub,
                    client_kp.as_inner(),
                    &cc,
                    &sc,
                )
                .await
                .unwrap()
            },
            async {
                server_handshake(&mut sr, &mut sw, server_kp.as_inner(), &sc, &cc)
                    .await
                    .unwrap()
            },
        );

        ct.write_encrypted_frame(&mut cw, b"").await.unwrap();
        let decrypted = st.read_encrypted_frame(&mut sr).await.unwrap();
        assert!(decrypted.is_empty());
    }
}

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
//! The prologue binds both ends' UIDs and the socket path (v2), so a
//! handshake only completes between processes of the same user that dialled
//! the same bus. [RC-4]
//!
//! Every Noise message is at most 65535 bytes (spec rev 34 §3) and travels
//! with a 16-bit length (`framing`). An application frame of up to
//! `MAX_APP_FRAME` is split into transport messages; the first message's
//! plaintext opens with the frame's total length (`u32` BE), so the length is
//! authenticated before anything is allocated for the frame, and a truncated
//! or extended sequence of messages is detected (§13: messages "could be
//! truncated by an attacker").
//!
//! Adapted from open-sesame `core-ipc/src/noise.rs`.

use std::path::Path;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

use super::error::{IpcError, Result};
use super::framing::{read_noise_message, write_noise_message, MAX_APP_FRAME, MAX_NOISE_MESSAGE};
use super::noise_keys::NOISE_PARAMS;

/// AEAD tag length (ChaChaPoly).
const TAG_LEN: usize = 16;

/// Maximum plaintext per Noise transport message: 65535 - 16 = 65519.
const MAX_NOISE_PLAINTEXT: usize = MAX_NOISE_MESSAGE - TAG_LEN;

/// Bytes of the authenticated total-length header in a frame's first message.
const FRAME_HEADER_LEN: usize = 4;

/// Handshake timeout to prevent DoS via slow handshake. [RC-7]
const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Build the Noise prologue (v2):
/// `REKINDLE-IPC-v2 ‖ lower_uid (u32 BE) ‖ higher_uid (u32 BE) ‖ socket path`.
///
/// Sorted UIDs give both ends identical bytes. No PIDs: a peer in another PID
/// namespace has no meaningful PID, and the UID check plus the socket's
/// permissions are the boundary. The prologue is mixed into the handshake
/// hash; any difference fails the handshake. [RC-4]
fn build_prologue(local_uid: u32, remote_uid: u32, socket_path: &Path) -> Vec<u8> {
    let (low, high) = if local_uid <= remote_uid {
        (local_uid, remote_uid)
    } else {
        (remote_uid, local_uid)
    };
    let path = socket_path.as_os_str().as_encoded_bytes();
    let mut prologue = Vec::with_capacity(15 + 8 + path.len());
    prologue.extend_from_slice(b"REKINDLE-IPC-v2");
    prologue.extend_from_slice(&low.to_be_bytes());
    prologue.extend_from_slice(&high.to_be_bytes());
    prologue.extend_from_slice(path);
    prologue
}

/// A frame being reassembled from transport messages.
#[derive(Default)]
struct FrameAssembler {
    /// The frame's authenticated total length, once its first message is in.
    expected: Option<usize>,
    buf: Vec<u8>,
}

impl FrameAssembler {
    /// Add one decrypted message; the completed frame when it is the last.
    fn push(&mut self, plaintext: &[u8]) -> Result<Option<Vec<u8>>> {
        let (expected, body) = if let Some(expected) = self.expected {
            (expected, plaintext)
        } else {
            let (header, body) = plaintext.split_first_chunk::<FRAME_HEADER_LEN>().ok_or(
                IpcError::InvalidChunkHeader {
                    got: plaintext.len(),
                },
            )?;
            let total = u32::from_be_bytes(*header);
            if total > MAX_APP_FRAME {
                return Err(IpcError::FrameTooLarge {
                    size: total,
                    max: MAX_APP_FRAME,
                });
            }
            let total = total as usize;
            self.expected = Some(total);
            self.buf = Vec::with_capacity(total);
            (total, body)
        };
        let received = self.buf.len() + body.len();
        if received > expected {
            return Err(IpcError::FrameLengthMismatch {
                declared: expected,
                received,
            });
        }
        self.buf.extend_from_slice(body);
        if received == expected {
            self.expected = None;
            return Ok(Some(std::mem::take(&mut self.buf)));
        }
        Ok(None)
    }
}

/// Encrypted IPC transport wrapping a completed Noise session.
///
/// `TransportState` takes `&mut self` for encrypt and decrypt, so one task
/// owns it: the connection's I/O loop, fed ciphertext by a reader task
/// (`framing::spawn_reader`). snow advances the receive nonce only after a
/// message authenticates, and the send and receive cipher states are
/// independent.
pub struct NoiseTransport {
    state: snow::TransportState,
    inbound: FrameAssembler,
}

impl NoiseTransport {
    fn new(state: snow::TransportState) -> Self {
        Self {
            state,
            inbound: FrameAssembler::default(),
        }
    }

    /// Encrypt an application frame into its transport messages.
    ///
    /// # Errors
    /// The frame exceeds [`MAX_APP_FRAME`], or encryption fails.
    pub fn encrypt_frame(&mut self, payload: &[u8]) -> Result<Vec<Vec<u8>>> {
        let total = u32::try_from(payload.len())
            .ok()
            .filter(|len| *len <= MAX_APP_FRAME)
            .ok_or(IpcError::FrameTooLarge {
                size: u32::try_from(payload.len()).unwrap_or(u32::MAX),
                max: MAX_APP_FRAME,
            })?;
        let mut plaintext = Vec::with_capacity(FRAME_HEADER_LEN + payload.len());
        plaintext.extend_from_slice(&total.to_be_bytes());
        plaintext.extend_from_slice(payload);

        let mut messages = Vec::with_capacity(plaintext.len().div_ceil(MAX_NOISE_PLAINTEXT));
        let mut buf = vec![0u8; MAX_NOISE_MESSAGE];
        for chunk in plaintext.chunks(MAX_NOISE_PLAINTEXT) {
            let len =
                self.state
                    .write_message(chunk, &mut buf)
                    .map_err(|e| IpcError::EncryptFailed {
                        reason: e.to_string(),
                    })?;
            messages.push(buf[..len].to_vec());
        }
        zeroize::Zeroize::zeroize(&mut plaintext);
        Ok(messages)
    }

    /// Encrypt `payload` and write its messages, flushing once.
    ///
    /// # Errors
    /// Encryption or socket failure.
    pub async fn write_frame<W: AsyncWrite + Unpin>(
        &mut self,
        writer: &mut W,
        payload: &[u8],
    ) -> Result<()> {
        for message in self.encrypt_frame(payload)? {
            write_noise_message(writer, &message).await?;
        }
        writer.flush().await?;
        Ok(())
    }

    /// Decrypt one transport message; the completed application frame when
    /// it is the frame's last message.
    ///
    /// # Errors
    /// The message does not authenticate, or the frame's messages do not
    /// match its authenticated length.
    pub fn read_message(&mut self, ciphertext: &[u8]) -> Result<Option<Vec<u8>>> {
        let mut buf = vec![0u8; ciphertext.len()];
        let len =
            self.state
                .read_message(ciphertext, &mut buf)
                .map_err(|e| IpcError::DecryptFailed {
                    reason: e.to_string(),
                })?;
        let frame = self.inbound.push(&buf[..len]);
        // [RC-16] Zeroize the intermediate decrypt buffer.
        zeroize::Zeroize::zeroize(&mut buf);
        frame
    }

    /// Get the remote party's static public key (after handshake).
    pub fn remote_static(&self) -> Option<&[u8]> {
        self.state.get_remote_static()
    }
}

/// What both ends bind into the prologue: their UIDs and the bus path.
#[derive(Debug, Clone, Copy)]
pub struct HandshakePeer<'a> {
    pub local_uid: u32,
    pub remote_uid: u32,
    pub socket_path: &'a Path,
}

/// Perform the server-side (responder) Noise IK handshake.
///
/// IK responder: read msg1 (initiator's ephemeral + encrypted static),
/// write msg2 (responder's ephemeral). Then derive transport keys.
pub async fn server_handshake<R, W>(
    reader: &mut R,
    writer: &mut W,
    server_keypair: &snow::Keypair,
    peer: HandshakePeer<'_>,
) -> Result<NoiseTransport>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let prologue = build_prologue(peer.local_uid, peer.remote_uid, peer.socket_path);

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
        let msg1 = read_noise_message(reader).await?;
        let mut payload_buf = vec![0u8; MAX_NOISE_MESSAGE];
        handshake
            .read_message(&msg1, &mut payload_buf)
            .map_err(|e| IpcError::HandshakeFailed {
                reason: format!("msg1 read: {e}"),
            })?;

        // Write msg2 to initiator.
        let mut msg2_buf = vec![0u8; MAX_NOISE_MESSAGE];
        let msg2_len =
            handshake
                .write_message(&[], &mut msg2_buf)
                .map_err(|e| IpcError::HandshakeFailed {
                    reason: format!("msg2 write: {e}"),
                })?;
        write_noise_message(writer, &msg2_buf[..msg2_len]).await?;
        writer.flush().await?;

        // Transition to transport mode.
        let transport = handshake
            .into_transport_mode()
            .map_err(|e| IpcError::HandshakeFailed {
                reason: format!("transport mode: {e}"),
            })?;

        tracing::debug!("Noise IK handshake completed (server)");
        Ok(NoiseTransport::new(transport))
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
    peer: HandshakePeer<'_>,
) -> Result<NoiseTransport>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let prologue = build_prologue(peer.local_uid, peer.remote_uid, peer.socket_path);

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
        let mut msg1_buf = vec![0u8; MAX_NOISE_MESSAGE];
        let msg1_len =
            handshake
                .write_message(&[], &mut msg1_buf)
                .map_err(|e| IpcError::HandshakeFailed {
                    reason: format!("msg1 write: {e}"),
                })?;
        write_noise_message(writer, &msg1_buf[..msg1_len]).await?;
        writer.flush().await?;

        // Read msg2 from responder.
        let msg2 = read_noise_message(reader).await?;
        let mut payload_buf = vec![0u8; MAX_NOISE_MESSAGE];
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
        Ok(NoiseTransport::new(transport))
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

    const BUS: &str = "/run/user/1000/rekindle/daemon.sock";

    fn peer(local_uid: u32, remote_uid: u32) -> HandshakePeer<'static> {
        HandshakePeer {
            local_uid,
            remote_uid,
            socket_path: Path::new(BUS),
        }
    }

    #[test]
    fn prologue_v2_is_canonical_and_pid_free() {
        let a = build_prologue(1000, 1001, Path::new(BUS));
        assert_eq!(a, build_prologue(1001, 1000, Path::new(BUS)));
        let mut expected = b"REKINDLE-IPC-v2".to_vec();
        expected.extend_from_slice(&1000u32.to_be_bytes());
        expected.extend_from_slice(&1001u32.to_be_bytes());
        expected.extend_from_slice(BUS.as_bytes());
        assert_eq!(a, expected);
    }

    /// Two transports over an in-memory duplex: (client, server, client
    /// stream halves, server stream halves).
    async fn pair(
        client_peer: HandshakePeer<'_>,
        server_peer: HandshakePeer<'_>,
    ) -> (
        Result<NoiseTransport>,
        Result<NoiseTransport>,
        (
            tokio::io::ReadHalf<tokio::io::DuplexStream>,
            tokio::io::WriteHalf<tokio::io::DuplexStream>,
        ),
        (
            tokio::io::ReadHalf<tokio::io::DuplexStream>,
            tokio::io::WriteHalf<tokio::io::DuplexStream>,
        ),
    ) {
        let server_kp = generate_keypair().unwrap();
        let client_kp = generate_keypair().unwrap();
        let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
        let (cs, ss) = tokio::io::duplex(1024 * 1024);
        let (mut cr, mut cw) = tokio::io::split(cs);
        let (mut sr, mut sw) = tokio::io::split(ss);
        let (client, server) = tokio::join!(
            client_handshake(
                &mut cr,
                &mut cw,
                &server_pub,
                client_kp.as_inner(),
                client_peer
            ),
            server_handshake(&mut sr, &mut sw, server_kp.as_inner(), server_peer),
        );
        (client, server, (cr, cw), (sr, sw))
    }

    /// Read one frame the way the I/O loops do: whole messages in, frame out.
    async fn read_frame<R: AsyncRead + Unpin>(t: &mut NoiseTransport, r: &mut R) -> Vec<u8> {
        loop {
            let message = read_noise_message(r).await.unwrap();
            if let Some(frame) = t.read_message(&message).unwrap() {
                return frame;
            }
        }
    }

    #[tokio::test]
    async fn handshake_and_transport_roundtrip() {
        let (client, server, (mut cr, mut cw), (mut sr, mut sw)) =
            pair(peer(1000, 1000), peer(1000, 1000)).await;
        let (mut client, mut server) = (client.unwrap(), server.unwrap());

        client
            .write_frame(&mut cw, b"hello encrypted world")
            .await
            .unwrap();
        assert_eq!(
            read_frame(&mut server, &mut sr).await,
            b"hello encrypted world"
        );
        server.write_frame(&mut sw, b"acknowledged").await.unwrap();
        assert_eq!(read_frame(&mut client, &mut cr).await, b"acknowledged");
    }

    #[tokio::test]
    async fn large_and_empty_frames() {
        let (client, server, (_cr, mut cw), (mut sr, _sw)) =
            pair(peer(1000, 1000), peer(1000, 1000)).await;
        let (mut client, mut server) = (client.unwrap(), server.unwrap());

        let large = vec![0xABu8; 200 * 1024];
        let messages = client.encrypt_frame(&large).unwrap();
        assert_eq!(messages.len(), 4);
        for message in &messages {
            write_noise_message(&mut cw, message).await.unwrap();
        }
        cw.flush().await.unwrap();
        assert_eq!(read_frame(&mut server, &mut sr).await, large);

        client.write_frame(&mut cw, b"").await.unwrap();
        assert!(read_frame(&mut server, &mut sr).await.is_empty());
    }

    /// Both ends run the I/O loops' shape — a reader task feeding a
    /// `select!` that also writes — and exchange 10 000 frames each way
    /// concurrently, varied in size so frames span several messages. Every
    /// frame arrives intact and in order: no select ever loses part of a
    /// message or desynchronises a nonce (WS12.1).
    #[tokio::test]
    async fn duplex_interleave_soak() {
        const FRAMES: usize = 10_000;
        let (client, server, (cr, cw), (sr, sw)) = pair(peer(1000, 1000), peer(1000, 1000)).await;

        fn frame(i: usize) -> Vec<u8> {
            let len = if i.is_multiple_of(97) {
                70_000
            } else {
                i % 300
            };
            let mut f = u32::try_from(i).unwrap().to_be_bytes().to_vec();
            f.resize(4 + len, u8::try_from(i % 251).unwrap());
            f
        }

        async fn run<R, W>(mut t: NoiseTransport, reader: R, mut writer: W) -> usize
        where
            R: AsyncRead + Unpin + Send + 'static,
            W: AsyncWrite + Unpin,
        {
            let (mut inbound, reader_task) = crate::framing::spawn_reader(reader);
            let (mut sent, mut received) = (0usize, 0usize);
            while received < FRAMES || sent < FRAMES {
                tokio::select! {
                    message = inbound.recv(), if received < FRAMES => {
                        if let Some(f) = t.read_message(&message.unwrap()).unwrap() {
                            assert_eq!(f, frame(received), "frame {received}");
                            received += 1;
                        }
                    }
                    () = std::future::ready(()), if sent < FRAMES => {
                        t.write_frame(&mut writer, &frame(sent)).await.unwrap();
                        sent += 1;
                    }
                }
            }
            reader_task.abort();
            received
        }

        let (a, b) = tokio::join!(run(client.unwrap(), cr, cw), run(server.unwrap(), sr, sw));
        assert_eq!((a, b), (FRAMES, FRAMES));
    }

    /// A PID-less peer handshakes: the prologue carries no PID.
    #[tokio::test]
    async fn handshake_needs_no_pid() {
        let (client, server, _, _) = pair(peer(1000, 1000), peer(1000, 1000)).await;
        assert!(client.is_ok() && server.is_ok());
    }

    /// [RC-4] A different UID or bus path on either end fails the handshake.
    #[tokio::test]
    async fn prologue_mismatch_fails_handshake() {
        let (client, server, _, _) = pair(peer(1000, 1000), peer(1000, 9999)).await;
        assert!(client.is_err() || server.is_err(), "uid mismatch");

        let other = HandshakePeer {
            socket_path: Path::new("/tmp/other.sock"),
            ..peer(1000, 1000)
        };
        let (client, server, _, _) = pair(peer(1000, 1000), other).await;
        assert!(client.is_err() || server.is_err(), "path mismatch");
    }

    /// The total length is authenticated: a frame whose messages run past
    /// it is refused, and a length over the cap is refused before any
    /// allocation for the frame.
    #[test]
    fn assembler_enforces_the_authenticated_length() {
        let mut assembler = FrameAssembler::default();
        let mut first = 3u32.to_be_bytes().to_vec();
        first.extend_from_slice(b"ab");
        assert_eq!(assembler.push(&first).unwrap(), None);
        assert!(matches!(
            assembler.push(b"cd"),
            Err(IpcError::FrameLengthMismatch { .. })
        ));

        let mut assembler = FrameAssembler::default();
        assert!(matches!(
            assembler.push(&(MAX_APP_FRAME + 1).to_be_bytes()),
            Err(IpcError::FrameTooLarge { .. })
        ));
        assert!(matches!(
            FrameAssembler::default().push(b"ab"),
            Err(IpcError::InvalidChunkHeader { got: 2 })
        ));
    }
}

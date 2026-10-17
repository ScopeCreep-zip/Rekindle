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
/// `REKINDLE-IPC-v3 ‖ lower_uid (u32 BE) ‖ higher_uid (u32 BE) ‖ socket path`.
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
    prologue.extend_from_slice(b"REKINDLE-IPC-v3");
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
    server_handshake_as(
        reader,
        writer,
        server_keypair,
        peer,
        crate::message::BUILD_ID,
    )
    .await
}

/// The server handshake for a given build id: msg1 carries the client's,
/// msg2 answers with ours, and a mismatch is refused once msg2 is sent, so
/// the client learns both ids.
async fn server_handshake_as<R, W>(
    reader: &mut R,
    writer: &mut W,
    server_keypair: &snow::Keypair,
    peer: HandshakePeer<'_>,
    build_id: &str,
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
        let client_len = handshake
            .read_message(&msg1, &mut payload_buf)
            .map_err(|e| IpcError::HandshakeFailed {
                reason: format!("msg1 read: {e}"),
            })?;
        let client_build = String::from_utf8_lossy(&payload_buf[..client_len]).into_owned();

        // Write msg2 to initiator, carrying our build id.
        let mut msg2_buf = vec![0u8; MAX_NOISE_MESSAGE];
        let msg2_len = handshake
            .write_message(build_id.as_bytes(), &mut msg2_buf)
            .map_err(|e| IpcError::HandshakeFailed {
                reason: format!("msg2 write: {e}"),
            })?;
        write_noise_message(writer, &msg2_buf[..msg2_len]).await?;
        writer.flush().await?;
        if client_build != build_id {
            return Err(IpcError::BuildMismatch {
                client: client_build,
                host: build_id.to_owned(),
            });
        }

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
    client_handshake_as(
        reader,
        writer,
        server_public_key,
        client_keypair,
        peer,
        crate::message::BUILD_ID,
    )
    .await
}

/// The client handshake for a given build id: msg1 carries ours, msg2 the
/// host's; a mismatch is [`IpcError::BuildMismatch`].
async fn client_handshake_as<R, W>(
    reader: &mut R,
    writer: &mut W,
    server_public_key: &[u8; 32],
    client_keypair: &snow::Keypair,
    peer: HandshakePeer<'_>,
    build_id: &str,
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
        let msg1_len = handshake
            .write_message(build_id.as_bytes(), &mut msg1_buf)
            .map_err(|e| IpcError::HandshakeFailed {
                reason: format!("msg1 write: {e}"),
            })?;
        write_noise_message(writer, &msg1_buf[..msg1_len]).await?;
        writer.flush().await?;

        // Read msg2 from responder.
        let msg2 = read_noise_message(reader).await?;
        let mut payload_buf = vec![0u8; MAX_NOISE_MESSAGE];
        let host_len = handshake
            .read_message(&msg2, &mut payload_buf)
            .map_err(|e| IpcError::HandshakeFailed {
                reason: format!("msg2 read: {e}"),
            })?;
        let host_build = String::from_utf8_lossy(&payload_buf[..host_len]).into_owned();
        if host_build != build_id {
            return Err(IpcError::BuildMismatch {
                client: build_id.to_owned(),
                host: host_build,
            });
        }

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
#[path = "noise_tests.rs"]
mod tests;

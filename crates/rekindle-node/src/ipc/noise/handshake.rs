//! Noise IK handshake functions (server and client).
//!
//! Both functions return `NoiseTransport` containing a `NoiseWriter`,
//! `NoiseReader`, and the handshake hash for bulk cipher derivation.
//!
//! The handshake completes with `into_stateless_transport_mode()` which
//! produces `StatelessTransportState`. This type takes `&self` for both
//! encrypt and decrypt with explicit nonces, enabling the writer and
//! reader to operate on independent tasks without `&mut self` conflicts.

use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::ipc::error::{IpcError, Result};
use crate::ipc::framing::{read_frame, write_frame};
use crate::ipc::noise_keys::NOISE_PARAMS;
use crate::ipc::transport::PeerCredentials;

use super::{build_prologue, NoiseTransport, MAX_NOISE_PLAINTEXT, HANDSHAKE_TIMEOUT};
use super::reader::NoiseReader;
use super::writer::NoiseWriter;

/// Perform the server-side (responder) Noise IK handshake.
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

    let mut handshake = snow::Builder::new(
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
        let msg1 = read_frame(reader).await?;
        tracing::debug!(msg1_len = msg1.len(), "server handshake: msg1 received");
        let mut payload_buf = [0u8; 256];
        handshake
            .read_message(&msg1, &mut payload_buf)
            .map_err(|e| IpcError::HandshakeFailed {
                reason: format!("msg1 read: {e}"),
            })?;

        let mut msg2_buf = [0u8; 256];
        let msg2_len = handshake
            .write_message(&[], &mut msg2_buf)
            .map_err(|e| IpcError::HandshakeFailed {
                reason: format!("msg2 write: {e}"),
            })?;
        tracing::debug!(msg2_len, "server handshake: msg2 sending");
        write_frame(writer, &msg2_buf[..msg2_len]).await?;
        use tokio::io::AsyncWriteExt;
        writer.flush().await.map_err(|e| IpcError::HandshakeFailed {
            reason: format!("msg2 flush: {e}"),
        })?;

        let handshake_hash: [u8; 32] = handshake
            .get_handshake_hash()
            .try_into()
            .map_err(|_| IpcError::HandshakeFailed {
                reason: "handshake hash not 32 bytes".to_string(),
            })?;

        let remote_static = handshake.get_remote_static().map(<[u8]>::to_vec);

        let transport = handshake
            .into_stateless_transport_mode()
            .map_err(|e| IpcError::HandshakeFailed {
                reason: format!("stateless transport mode: {e}"),
            })?;

        tracing::debug!("Noise IK handshake completed (server, stateless)");

        let state = Arc::new(transport);
        Ok(NoiseTransport {
            writer: NoiseWriter {
                state: Arc::clone(&state),
                enc_buf: Vec::with_capacity(MAX_NOISE_PLAINTEXT + 16),
                send_nonce: AtomicU64::new(0),
            },
            reader: NoiseReader {
                state,
                read_buf: bytes::BytesMut::with_capacity(4096),
                dec_buf: Vec::with_capacity(MAX_NOISE_PLAINTEXT + 16),
                remote_static,
                recv_nonce: AtomicU64::new(0),
            },
            handshake_hash: Some(zeroize::Zeroizing::new(handshake_hash)),
        })
    })
    .await
    .map_err(|_| IpcError::HandshakeTimeout {
        timeout_ms: HANDSHAKE_TIMEOUT.as_secs() * 1000,
    })?
}

/// Perform the client-side (initiator) Noise IK handshake.
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

    let mut handshake = snow::Builder::new(
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
        let mut msg1_buf = [0u8; 256];
        let msg1_len = handshake
            .write_message(&[], &mut msg1_buf)
            .map_err(|e| IpcError::HandshakeFailed {
                reason: format!("msg1 write: {e}"),
            })?;
        tracing::debug!(msg1_len, "client handshake: msg1 sending");
        write_frame(writer, &msg1_buf[..msg1_len]).await?;
        use tokio::io::AsyncWriteExt;
        writer.flush().await.map_err(|e| IpcError::HandshakeFailed {
            reason: format!("msg1 flush: {e}"),
        })?;

        let msg2 = read_frame(reader).await?;
        tracing::debug!(msg2_len = msg2.len(), "client handshake: msg2 received");
        let mut payload_buf = [0u8; 256];
        handshake
            .read_message(&msg2, &mut payload_buf)
            .map_err(|e| IpcError::HandshakeFailed {
                reason: format!("msg2 read: {e}"),
            })?;

        let handshake_hash: [u8; 32] = handshake
            .get_handshake_hash()
            .try_into()
            .map_err(|_| IpcError::HandshakeFailed {
                reason: "handshake hash not 32 bytes".to_string(),
            })?;

        let remote_static = handshake.get_remote_static().map(<[u8]>::to_vec);

        let transport = handshake
            .into_stateless_transport_mode()
            .map_err(|e| IpcError::HandshakeFailed {
                reason: format!("stateless transport mode: {e}"),
            })?;

        tracing::debug!("Noise IK handshake completed (client, stateless)");

        let state = Arc::new(transport);
        Ok(NoiseTransport {
            writer: NoiseWriter {
                state: Arc::clone(&state),
                enc_buf: Vec::with_capacity(MAX_NOISE_PLAINTEXT + 16),
                send_nonce: AtomicU64::new(0),
            },
            reader: NoiseReader {
                state,
                read_buf: bytes::BytesMut::with_capacity(4096),
                dec_buf: Vec::with_capacity(MAX_NOISE_PLAINTEXT + 16),
                remote_static,
                recv_nonce: AtomicU64::new(0),
            },
            handshake_hash: Some(zeroize::Zeroizing::new(handshake_hash)),
        })
    })
    .await
    .map_err(|_| IpcError::HandshakeTimeout {
        timeout_ms: HANDSHAKE_TIMEOUT.as_secs() * 1000,
    })?
}

//! Noise IK handshake functions (server and client).
//!
//! Both return `NoiseTransport` containing a `NoiseWriter`, `NoiseReader`,
//! and the handshake hash for bulk cipher derivation.
//!
//! Completes with `into_stateless_transport_mode()` producing
//! `StatelessTransportState` — takes `&self` for both encrypt and
//! decrypt with explicit nonces, enabling writer and reader to operate
//! on independent tasks without `&mut self` conflicts.

use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::error::{IpcError, IpcResult};
use crate::frame::codec::{read_frame, write_frame, MAX_FRAME_SIZE};
use crate::noise::keys::NOISE_PARAMS;
use crate::socket::PeerCredentials;

use super::reader::NoiseReader;
use super::writer::NoiseWriter;
use super::{build_prologue, NoiseTransport, MAX_NOISE_PLAINTEXT, WIRE_OVERHEAD_PER_CHUNK};

/// Server-side (responder) Noise IK handshake.
///
/// Two messages: client->server (msg1), server->client (msg2).
/// `timeout` controls the maximum wall time for the handshake exchange.
/// `max_frame_size` is the application-level payload limit from config.
/// It is stored in the resulting NoiseWriter (for outbound enforcement)
/// and derived into a wire-level limit on the NoiseReader (accounting
/// for transport overhead).
pub async fn server_handshake<R, W>(
    reader: &mut R,
    writer: &mut W,
    server_keypair: &snow::Keypair,
    local_creds: &PeerCredentials,
    remote_creds: &PeerCredentials,
    timeout: std::time::Duration,
    max_frame_size: u32,
) -> IpcResult<NoiseTransport>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let prologue = build_prologue(local_creds, remote_creds);

    let mut hs = super::resolver::noise_builder(NOISE_PARAMS)
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

    tokio::time::timeout(timeout, async {
        // msg1: client -> server
        let msg1 = read_frame(reader, MAX_FRAME_SIZE).await?;
        let mut payload_buf = [0u8; 256];
        hs.read_message(&msg1, &mut payload_buf)
            .map_err(|e| IpcError::HandshakeFailed {
                reason: format!("msg1 read: {e}"),
            })?;

        // msg2: server -> client
        let mut msg2_buf = [0u8; 256];
        let msg2_len = hs
            .write_message(&[], &mut msg2_buf)
            .map_err(|e| IpcError::HandshakeFailed {
                reason: format!("msg2 write: {e}"),
            })?;
        write_frame(writer, &msg2_buf[..msg2_len]).await?;
        tokio::io::AsyncWriteExt::flush(writer)
            .await
            .map_err(|e| IpcError::HandshakeFailed {
                reason: format!("msg2 flush: {e}"),
            })?;

        finish_handshake(hs, max_frame_size)
    })
    .await
    .map_err(|_| IpcError::HandshakeTimeout {
        timeout_ms: timeout.as_millis() as u64,
    })?
}

/// Client-side (initiator) Noise IK handshake.
/// `timeout` controls the maximum wall time for the handshake exchange.
/// `max_frame_size` is the application-level payload limit from config.
pub async fn client_handshake<R, W>(
    reader: &mut R,
    writer: &mut W,
    server_public_key: &[u8; 32],
    client_keypair: &snow::Keypair,
    local_creds: &PeerCredentials,
    remote_creds: &PeerCredentials,
    timeout: std::time::Duration,
    max_frame_size: u32,
) -> IpcResult<NoiseTransport>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let prologue = build_prologue(local_creds, remote_creds);

    let mut hs = super::resolver::noise_builder(NOISE_PARAMS)
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

    tokio::time::timeout(timeout, async {
        // msg1: client -> server
        let mut msg1_buf = [0u8; 256];
        let msg1_len = hs
            .write_message(&[], &mut msg1_buf)
            .map_err(|e| IpcError::HandshakeFailed {
                reason: format!("msg1 write: {e}"),
            })?;
        write_frame(writer, &msg1_buf[..msg1_len]).await?;
        tokio::io::AsyncWriteExt::flush(writer)
            .await
            .map_err(|e| IpcError::HandshakeFailed {
                reason: format!("msg1 flush: {e}"),
            })?;

        // msg2: server -> client
        let msg2 = read_frame(reader, MAX_FRAME_SIZE).await?;
        let mut payload_buf = [0u8; 256];
        hs.read_message(&msg2, &mut payload_buf)
            .map_err(|e| IpcError::HandshakeFailed {
                reason: format!("msg2 read: {e}"),
            })?;

        finish_handshake(hs, max_frame_size)
    })
    .await
    .map_err(|_| IpcError::HandshakeTimeout {
        timeout_ms: timeout.as_millis() as u64,
    })?
}

/// Common handshake finalization: extract hash, remote static, transition
/// to stateless transport mode.
///
/// `max_frame_size` is the application-level payload limit from config.
/// Both reader and writer use `max_frame_size + WIRE_OVERHEAD_PER_CHUNK`
/// (25 bytes: 9 app framing + 16 AEAD tag) as the wire-level per-chunk
/// limit. The 4-byte chunk count header is a separate read_frame call
/// and is not part of the per-chunk limit.
///
/// The application-level check (without overhead) is enforced at:
/// - Client: `send_frame` checks before tagging → graceful rejection
/// - Server: `RecvControl` checks after decrypt → graceful rejection
///
/// The wire-level check is defense-in-depth — a rejection at this layer
/// means the framing is corrupt or the peer is malicious, and the
/// connection is torn down.
fn finish_handshake(hs: snow::HandshakeState, max_frame_size: u32) -> IpcResult<NoiseTransport> {
    let handshake_hash: [u8; 32] = hs
        .get_handshake_hash()
        .try_into()
        .map_err(|_| IpcError::HandshakeFailed {
            reason: "handshake hash not 32 bytes".into(),
        })?;

    let remote_static = hs.get_remote_static().map(<[u8]>::to_vec);

    let state = Arc::new(
        hs.into_stateless_transport_mode()
            .map_err(|e| IpcError::HandshakeFailed {
                reason: format!("stateless transport: {e}"),
            })?,
    );

    // Wire-level limit: application payload + per-chunk overhead (25 bytes).
    // The 4-byte chunk count header is a separate read_frame call and is
    // not part of the per-chunk limit.
    // Saturating add prevents overflow when max_frame_size is near u32::MAX.
    let wire_max = max_frame_size.saturating_add(WIRE_OVERHEAD_PER_CHUNK);

    Ok(NoiseTransport {
        writer: NoiseWriter {
            state: Arc::clone(&state),
            enc_buf: Vec::with_capacity(MAX_NOISE_PLAINTEXT + 16),
            send_nonce: AtomicU64::new(0),
            max_frame_size: wire_max, // same wire-level limit as reader
        },
        reader: NoiseReader {
            state,
            dec_buf: Vec::with_capacity(MAX_NOISE_PLAINTEXT + 16),
            remote_static,
            recv_nonce: AtomicU64::new(0),
            max_frame_size: wire_max,
        },
        handshake_hash: Some(zeroize::Zeroizing::new(handshake_hash)),
    })
}

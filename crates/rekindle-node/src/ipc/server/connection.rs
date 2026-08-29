//! Per-connection handler: Noise IK handshake, then encrypted frame I/O.

use std::sync::Arc;
use std::time::Instant;

use tokio::net::UnixStream;
use tokio::sync::mpsc;

use super::routing;
use super::{ConnectionState, ServerState, RATE_LIMIT_MAX_TOKENS};

use crate::ipc::message::SecurityLevel;
use crate::ipc::noise::{self, NoiseTransport};
use crate::ipc::transport::PeerCredentials;

/// Handle a single client connection: Noise handshake, then encrypted I/O.
///
/// Connection is registered in state AFTER handshake succeeds — no frames
/// can arrive on `tx` before the writer task is spawned. [RC-11]
pub(super) async fn handle_connection(
    state: Arc<ServerState>,
    conn_id: u64,
    stream: UnixStream,
    tx: mpsc::Sender<Vec<u8>>,
    mut outbound_rx: mpsc::Receiver<Vec<u8>>,
    peer_creds: PeerCredentials,
    keypair: Arc<snow::Keypair>,
) {
    let (reader, writer) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(reader);
    let mut writer = tokio::io::BufWriter::new(writer);

    let local_creds = PeerCredentials::local();
    let connected_at = Instant::now();

    // Noise IK handshake
    let mut transport: NoiseTransport = match noise::server_handshake(
        &mut reader,
        &mut writer,
        &keypair,
        &local_creds,
        &peer_creds,
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(
                conn_id,
                peer_pid = peer_creds.pid,
                error = %e,
                "Noise handshake failed"
            );
            return;
        }
    };

    // Extract client's X25519 static pubkey.
    let Some(key) = transport.remote_static() else {
        tracing::error!(conn_id, "no remote static key after handshake");
        return;
    };
    let Ok(client_pubkey): std::result::Result<[u8; 32], _> = key.try_into() else {
        tracing::error!(conn_id, "client pubkey not 32 bytes");
        return;
    };

    // Registry lookup: pubkey → (name, clearance).
    let (security_clearance, verified_name) = {
        let reg = state.registry.read().await;
        if let Some(identity) = reg.lookup(&client_pubkey) {
            let name = reg.lookup_name(&client_pubkey).map(str::to_owned);
            tracing::info!(
                conn_id,
                agent = name.as_deref().unwrap_or("unknown"),
                clearance = ?identity.security_level,
                "agent authenticated via registry"
            );
            (identity.security_level, name)
        } else {
            tracing::info!(
                conn_id,
                clearance = ?SecurityLevel::Open,
                "ephemeral client accepted (not in registry)"
            );
            (SecurityLevel::Open, None)
        }
    };

    // Register connection AFTER handshake.
    let conn = ConnectionState {
        agent_id: None,
        verified_name: verified_name.clone(),
        tx,
        peer: peer_creds,
        security_clearance,
        connected_at,
        rate_tokens: std::sync::atomic::AtomicU32::new(RATE_LIMIT_MAX_TOKENS),
        last_token_refill: std::sync::atomic::AtomicU64::new(0),
    };
    state.connections.write().await.insert(conn_id, conn);

    // Populate name_to_conn for unicast routing.
    if let Some(ref name) = verified_name {
        state
            .name_to_conn
            .write()
            .await
            .insert(name.clone(), conn_id);
    }

    // Multiplexed I/O loop. [RC-16] All buffers zeroized after use.
    loop {
        tokio::select! {
            result = transport.read_encrypted_frame(&mut reader) => {
                match result {
                    Ok(mut payload) => {
                        routing::route_frame(&state, conn_id, &payload).await;
                        zeroize::Zeroize::zeroize(&mut payload);
                    }
                    Err(e) => {
                        let session_ms = connected_at.elapsed().as_millis();
                        tracing::info!(
                            conn_id,
                            session_ms = %session_ms,
                            error = %e,
                            "client disconnected"
                        );
                        break;
                    }
                }
            }
            Some(mut payload) = outbound_rx.recv() => {
                let result = transport.write_encrypted_frame(&mut writer, &payload).await;
                zeroize::Zeroize::zeroize(&mut payload);
                if let Err(e) = result {
                    tracing::debug!(conn_id, error = %e, "write failed, closing");
                    break;
                }
            }
            else => break,
        }
    }

    // Cleanup: remove from all routing tables and broadcast disconnect event.
    let disconnected_name = {
        let conns = state.connections.read().await;
        match conns.get(&conn_id) {
            Some(c) => {
                let duration =
                    u64::try_from(c.connected_at.elapsed().as_millis()).unwrap_or(u64::MAX);
                tracing::info!(
                    conn_id,
                    agent = c.verified_name.as_deref().unwrap_or("ephemeral"),
                    peer_pid = c.peer.pid,
                    session_ms = duration,
                    "connection cleanup"
                );
                c.verified_name.clone()
            }
            None => None,
        }
    };
    state.connections.write().await.remove(&conn_id);
    state.event_router.write().remove_connection(conn_id);
    state
        .pending_requests
        .write()
        .await
        .retain(|_, cid| *cid != conn_id);
    state
        .name_to_conn
        .write()
        .await
        .retain(|_, cid| *cid != conn_id);

    // Log agent disconnect. Event delivery to subscribers is handled by the
    // subscription manager, not by broadcasting raw frames on the bus.
    if let Some(ref name) = disconnected_name {
        tracing::info!(
            agent = %name,
            conn_id,
            "agent disconnected"
        );
    }
}

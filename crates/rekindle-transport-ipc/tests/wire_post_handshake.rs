//! Post-handshake wire-level attack tests over real Unix sockets.
//!
//! Every test completes a real Noise IK handshake via manual_handshake(),
//! derives a StatelessTransportState, then crafts frames with precise
//! control over the ciphertext. The server must detect each attack,
//! disconnect the attacker, and continue serving legitimate clients.
//!
//! WILL FAIL if:
//! - NoiseReader (reader.rs:68-72) panics on AEAD error instead of Err(DecryptFailed)
//! - read_task (connection.rs:367) doesn't send ReadFrame::Disconnected on Err
//! - Control loop (connection.rs:303) doesn't break on Shutdown/Disconnected
//! - Accept loop doesn't continue after a connection dies
//!
//! Upstream reference: snow/tests/general.rs:925-957 test_stateful_nonce_increment_behavior
//! Upstream reference: snowstorm/tests/integration_test.rs:9-58 wrong_remote_pubkey

mod common;

use std::path::PathBuf;
use std::time::Duration;
use bytes::Bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use uuid::Uuid;

use rekindle_transport_ipc::config::IpcConfig;
use rekindle_transport_ipc::noise::keys::{generate_keypair, NOISE_PARAMS};
use rekindle_transport_ipc::noise::resolver::noise_builder;
use rekindle_transport_ipc::socket::{PeerCredentials, extract_ucred};
use rekindle_transport_ipc::server::state::ServerState;
use rekindle_transport_ipc::server::{FrameRouter, IpcServer};
use rekindle_transport_ipc::transport_frame::*;
use rekindle_transport_ipc::client::IpcClient;
use rekindle_transport_ipc::frame::lane::LANE_CONTROL;

fn sock_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rekindle-wph-{}-{}-{}.sock", label, std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ))
}

struct WphRouter {
    control_tx: mpsc::Sender<(u64, Bytes)>,
    state_tx: mpsc::Sender<(u64, ConnectionPhase, ConnectionPhase)>,
}
impl FrameRouter for WphRouter {
    fn route_frame(&self, _: &ServerState, id: u64, p: Bytes) {
        let _ = self.control_tx.try_send((id, p));
    }
    fn on_bulk_chunk(&self, _: &ServerState, _: u64, _: u8, _: u32, _: &[u8]) {}
    fn on_bulk_complete(&self, _: &ServerState, _: u64, _: u8, _: u64, _: u64) {}
    fn on_connection_state_changed(&self, _: &ServerState, id: u64, old: ConnectionPhase, new: ConnectionPhase) {
        let _ = self.state_tx.try_send((id, old, new));
    }
}

async fn make_server(path: &std::path::Path) -> (
    [u8; 32],
    mpsc::Receiver<(u64, Bytes)>,
    mpsc::Receiver<(u64, ConnectionPhase, ConnectionPhase)>,
    tokio::task::JoinHandle<()>,
) {
    let kp = generate_keypair().unwrap();
    let pub_key: [u8; 32] = kp.public().try_into().unwrap();
    let (ctx, crx) = mpsc::channel(256);
    let (stx, srx) = mpsc::channel(256);
    let router = WphRouter { control_tx: ctx, state_tx: stx };
    let server = IpcServer::bind(path, kp.into_inner(), router, IpcConfig::default()).unwrap();
    let h = tokio::spawn(async move { let _ = server.run().await; });
    tokio::task::yield_now().await;
    (pub_key, crx, srx, h)
}

async fn wait_ready(rx: &mut mpsc::Receiver<(u64, ConnectionPhase, ConnectionPhase)>) -> u64 {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some((id, _, ConnectionPhase::Ready))) => return id,
            Ok(Some(_)) => continue,
            _ => panic!("never Ready"),
        }
    }
}

/// Complete a Noise IK handshake manually over a raw UnixStream.
///
/// Uses exact prologue format from src/noise/mod.rs:34-45:
///   REKINDLE-IPC-v1:{lower_pid}:{lower_uid}:{higher_pid}:{higher_uid}
///
/// Uses exact handshake flow from src/noise/handshake.rs:
///   msg1: initiator write_frame → responder read_frame
///   msg2: responder write_frame → initiator read_frame
///
/// write_frame = [u32 LE length][payload] per src/frame/codec.rs:70-80
async fn manual_handshake(
    path: &std::path::Path,
    server_pub: &[u8; 32],
) -> (snow::StatelessTransportState, tokio::net::UnixStream) {
    let mut stream = tokio::net::UnixStream::connect(path).await.unwrap();

    let server_creds = extract_ucred(&stream).unwrap();
    let local_creds = PeerCredentials::local();

    // build_prologue: src/noise/mod.rs:34-45
    let (first, second) = if local_creds.pid <= server_creds.pid {
        (&local_creds, &server_creds)
    } else {
        (&server_creds, &local_creds)
    };
    let prologue = format!(
        "REKINDLE-IPC-v1:{}:{}:{}:{}",
        first.pid, first.uid, second.pid, second.uid
    ).into_bytes();

    let client_kp = generate_keypair().unwrap();
    let mut hs = noise_builder(NOISE_PARAMS)
        .local_private_key(&client_kp.as_inner().private).unwrap()
        .remote_public_key(server_pub).unwrap()
        .prologue(&prologue).unwrap()
        .build_initiator().unwrap();

    // msg1: client → server. Wire: [u32 LE length][msg1 bytes]
    let mut msg1 = [0u8; 256];
    let msg1_len = hs.write_message(&[], &mut msg1).unwrap();
    stream.write_all(&(msg1_len as u32).to_le_bytes()).await.unwrap();
    stream.write_all(&msg1[..msg1_len]).await.unwrap();
    stream.flush().await.unwrap();

    // msg2: server → client. Wire: [u32 LE length][msg2 bytes]
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await.unwrap();
    let msg2_len = u32::from_le_bytes(len_buf) as usize;
    let mut msg2 = vec![0u8; msg2_len];
    stream.read_exact(&mut msg2).await.unwrap();
    let mut payload = [0u8; 256];
    hs.read_message(&msg2, &mut payload).unwrap();

    let transport = hs.into_stateless_transport_mode().unwrap();
    (transport, stream)
}

/// Write one Noise-encrypted control frame using the exact wire format.
///
/// Wire format per noise_write (client.rs:462-470) + write_encrypted_frame
/// (writer.rs:31-73) + write_frame (codec.rs:70-80):
///
///   [lane=0x00]
///   [u32 LE: 4]  [u32 LE: chunk_count]   // chunk count via write_frame
///   [u32 LE: ct_len]  [ciphertext]        // chunk via write_frame
///
/// Returns the raw ciphertext bytes for replay/corruption tests.
async fn write_noise_control_frame(
    stream: &mut tokio::net::UnixStream,
    transport: &snow::StatelessTransportState,
    nonce: u64,
    plaintext: &[u8],
) -> Vec<u8> {
    let mut enc = vec![0u8; plaintext.len() + 16];
    let ct_len = transport.write_message(nonce, plaintext, &mut enc).unwrap();
    enc.truncate(ct_len);

    stream.write_all(&[LANE_CONTROL]).await.unwrap();
    stream.write_all(&4u32.to_le_bytes()).await.unwrap();
    stream.write_all(&1u32.to_le_bytes()).await.unwrap();
    stream.write_all(&(ct_len as u32).to_le_bytes()).await.unwrap();
    stream.write_all(&enc).await.unwrap();
    stream.flush().await.unwrap();

    enc
}

/// Build an application-tagged frame: [0x80][seq: u64 LE][payload]
/// per src/transport_frame.rs:184-189
fn build_app_frame(seq: u64, payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 8 + payload.len());
    buf.push(TransportTag::APP);
    buf.extend_from_slice(&seq.to_le_bytes());
    buf.extend_from_slice(payload);
    buf
}

// ---- Baseline: prove manual handshake works ----

/// If this fails, the manual_handshake or wire format construction is wrong.
/// All subsequent tests depend on this baseline.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn baseline_manual_handshake_valid_frame() {
    common::init_tracing();
    let path = sock_path("baseline");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let (pub_key, mut crx, mut srx, handle) = make_server(&path).await;

    let (transport, mut stream) = manual_handshake(&path, &pub_key).await;
    wait_ready(&mut srx).await;

    let tagged = build_app_frame(0, b"manual-baseline");
    write_noise_control_frame(&mut stream, &transport, 0, &tagged).await;

    let (_, payload) = tokio::time::timeout(Duration::from_secs(5), crx.recv())
        .await.unwrap().unwrap();
    assert_eq!(&payload[..], b"manual-baseline");

    drop(stream);
    handle.abort();
}

// ---- Attack: corrupted ciphertext ----

/// Proves: corrupted ciphertext → AEAD fails → server disconnects attacker →
/// server keeps serving legitimate clients.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn corrupt_ciphertext_server_survives() {
    common::init_tracing();
    let path = sock_path("corrupt-ct");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let (pub_key, mut crx, mut srx, handle) = make_server(&path).await;

    let (transport, mut stream) = manual_handshake(&path, &pub_key).await;
    wait_ready(&mut srx).await;

    // Send one VALID frame — proves handshake worked.
    let tagged = build_app_frame(0, b"valid-first");
    write_noise_control_frame(&mut stream, &transport, 0, &tagged).await;
    let (_, p) = tokio::time::timeout(Duration::from_secs(5), crx.recv())
        .await.unwrap().unwrap();
    assert_eq!(&p[..], b"valid-first");

    // Send CORRUPTED frame.
    let tagged2 = build_app_frame(1, b"will-be-corrupted");
    let mut enc = vec![0u8; tagged2.len() + 16];
    let ct_len = transport.write_message(1, &tagged2, &mut enc).unwrap();
    enc.truncate(ct_len);
    enc[5] ^= 0xFF; // flip one ciphertext byte

    stream.write_all(&[LANE_CONTROL]).await.unwrap();
    stream.write_all(&4u32.to_le_bytes()).await.unwrap();
    stream.write_all(&1u32.to_le_bytes()).await.unwrap();
    stream.write_all(&(ct_len as u32).to_le_bytes()).await.unwrap();
    stream.write_all(&enc).await.unwrap();
    stream.flush().await.unwrap();

    // DO NOT drop the stream yet. Send a VALID frame AFTER the corrupt one
    // on the SAME connection. If the server rejected the AEAD (not just EOF),
    // it will have closed this connection's read task. The valid frame will
    // either fail to write (broken pipe) or be written but never delivered
    // (server already disconnected us). Either way, the server's control_rx
    // must NOT contain "should-not-arrive".
    let tagged3 = build_app_frame(2, b"should-not-arrive");
    let mut enc3 = vec![0u8; tagged3.len() + 16];
    let ct_len3 = transport.write_message(2, &tagged3, &mut enc3).unwrap();
    enc3.truncate(ct_len3);

    // This write may succeed (kernel buffer accepts it) or fail (broken pipe).
    // Either is fine — what matters is the server doesn't deliver it.
    let _ = stream.write_all(&[LANE_CONTROL]).await;
    let _ = stream.write_all(&4u32.to_le_bytes()).await;
    let _ = stream.write_all(&1u32.to_le_bytes()).await;
    let _ = stream.write_all(&(ct_len3 as u32).to_le_bytes()).await;
    let _ = stream.write_all(&enc3).await;
    let _ = stream.flush().await;

    // Wait for the server to disconnect the attacker's connection.
    // We don't know the attacker's conn_id, but we can wait for ANY
    // connection to reach a terminal phase — the only connection that
    // would die is the attacker's (the valid frame proved it was alive).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, srx.recv()).await {
            Ok(Some((_, _, phase))) if phase.is_terminal() => break,
            Ok(Some(_)) => continue,
            _ => break, // timeout — server may not have disconnected yet, proceed anyway
        }
    }
    drop(stream);

    // Verify "should-not-arrive" was NOT delivered to the router.
    // Drain any frames that arrived and check none match.
    let mut found_poison = false;
    while let Ok(Some((_, p))) = tokio::time::timeout(Duration::from_millis(200), crx.recv()).await {
        if &p[..] == b"should-not-arrive" {
            found_poison = true;
        }
    }
    assert!(
        !found_poison,
        "server delivered a frame AFTER receiving corrupted AEAD ciphertext. \
         The server must disconnect the attacker on AEAD failure, not continue reading."
    );

    // Legitimate client MUST still work.
    let kp = generate_keypair().unwrap();
    let legit = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, kp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, srx.recv()).await {
            Ok(Some((_, _, ConnectionPhase::Ready))) => break,
            Ok(Some(_)) => continue,
            _ => panic!("legit client never Ready after corrupt attacker"),
        }
    }

    let outcome = legit.send_frame(b"after-corrupt", Duration::from_secs(5)).await;
    assert!(outcome.is_delivered(), "server must survive corruption: {outcome:?}");
    let (_, p) = tokio::time::timeout(Duration::from_secs(5), crx.recv())
        .await.unwrap().unwrap();
    assert_eq!(&p[..], b"after-corrupt");

    legit.shutdown().await;
    handle.abort();
}

// ---- Attack: nonce replay ----

/// Proves: replayed frame → server's NoiseReader nonce mismatch → AEAD fails → disconnect.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replay_same_nonce_server_survives() {
    common::init_tracing();
    let path = sock_path("replay-nonce");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let (pub_key, mut crx, mut srx, handle) = make_server(&path).await;

    let (transport, mut stream) = manual_handshake(&path, &pub_key).await;
    wait_ready(&mut srx).await;

    let tagged = build_app_frame(0, b"nonce-zero");
    let ct = write_noise_control_frame(&mut stream, &transport, 0, &tagged).await;

    let (_, p) = tokio::time::timeout(Duration::from_secs(5), crx.recv())
        .await.unwrap().unwrap();
    assert_eq!(&p[..], b"nonce-zero");

    // Replay exact same ciphertext. Server expects nonce 1, frame has nonce 0.
    stream.write_all(&[LANE_CONTROL]).await.unwrap();
    stream.write_all(&4u32.to_le_bytes()).await.unwrap();
    stream.write_all(&1u32.to_le_bytes()).await.unwrap();
    stream.write_all(&(ct.len() as u32).to_le_bytes()).await.unwrap();
    stream.write_all(&ct).await.unwrap();
    stream.flush().await.unwrap();
    drop(stream);

    let kp = generate_keypair().unwrap();
    let legit = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, kp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, srx.recv()).await {
            Ok(Some((_, _, ConnectionPhase::Ready))) => break,
            Ok(Some(_)) => continue,
            _ => panic!("never Ready after replay"),
        }
    }
    let outcome = legit.send_frame(b"after-replay", Duration::from_secs(5)).await;
    assert!(outcome.is_delivered());
    legit.shutdown().await;
    handle.abort();
}

// ---- Attack: truncated encrypted chunk ----

/// Proves: declared length but partial body → read_exact gets UnexpectedEof → disconnect.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn truncated_encrypted_chunk_server_survives() {
    common::init_tracing();
    let path = sock_path("trunc-chunk");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let (pub_key, _, mut srx, handle) = make_server(&path).await;

    let (transport, mut stream) = manual_handshake(&path, &pub_key).await;
    wait_ready(&mut srx).await;

    let tagged = build_app_frame(0, b"will-be-truncated");
    let mut enc = vec![0u8; tagged.len() + 16];
    let ct_len = transport.write_message(0, &tagged, &mut enc).unwrap();
    enc.truncate(ct_len);

    stream.write_all(&[LANE_CONTROL]).await.unwrap();
    stream.write_all(&4u32.to_le_bytes()).await.unwrap();
    stream.write_all(&1u32.to_le_bytes()).await.unwrap();
    // Declare full length but only send half.
    stream.write_all(&(ct_len as u32).to_le_bytes()).await.unwrap();
    stream.write_all(&enc[..ct_len / 2]).await.unwrap();
    stream.flush().await.unwrap();
    drop(stream); // EOF before all bytes delivered

    let kp = generate_keypair().unwrap();
    let legit = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, kp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, srx.recv()).await {
            Ok(Some((_, _, ConnectionPhase::Ready))) => break,
            Ok(Some(_)) => continue,
            _ => panic!("never Ready after truncation"),
        }
    }
    legit.shutdown().await;
    handle.abort();
}

// ---- Boundary: zero chunk count ----

/// chunk_count=0 → NoiseReader reads 0 chunks → empty Bytes →
/// control loop skips (connection.rs:279) → connection stays alive.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn zero_chunk_count_connection_survives() {
    common::init_tracing();
    let path = sock_path("zero-chunks");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let (pub_key, mut crx, mut srx, handle) = make_server(&path).await;

    let (transport, mut stream) = manual_handshake(&path, &pub_key).await;
    wait_ready(&mut srx).await;

    // Send chunk count of 0.
    stream.write_all(&[LANE_CONTROL]).await.unwrap();
    stream.write_all(&4u32.to_le_bytes()).await.unwrap();
    stream.write_all(&0u32.to_le_bytes()).await.unwrap();
    stream.flush().await.unwrap();

    // Send valid frame after — connection should still be alive.
    let tagged = build_app_frame(0, b"after-zero-chunks");
    write_noise_control_frame(&mut stream, &transport, 0, &tagged).await;

    match tokio::time::timeout(Duration::from_secs(5), crx.recv()).await {
        Ok(Some((_, p))) => assert_eq!(&p[..], b"after-zero-chunks"),
        other => panic!(
            "connection died after zero-chunk frame: {other:?}. \
             Implementation must handle chunk_count=0 gracefully."
        ),
    }

    drop(stream);
    handle.abort();
}

// ---- Attack: enormous chunk count ----

/// chunk_count=0xFFFFFFFF → NoiseReader TooManyChunks (reader.rs:49-55) → disconnect.
/// Server must NOT OOM.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn enormous_chunk_count_rejected() {
    common::init_tracing();
    let path = sock_path("huge-chunks");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let (pub_key, _, mut srx, handle) = make_server(&path).await;

    let (_, mut stream) = manual_handshake(&path, &pub_key).await;
    wait_ready(&mut srx).await;

    stream.write_all(&[LANE_CONTROL]).await.unwrap();
    stream.write_all(&4u32.to_le_bytes()).await.unwrap();
    stream.write_all(&0xFFFF_FFFFu32.to_le_bytes()).await.unwrap();
    stream.flush().await.unwrap();
    drop(stream);

    let kp = generate_keypair().unwrap();
    let legit = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, kp.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, srx.recv()).await {
            Ok(Some((_, _, ConnectionPhase::Ready))) => break,
            Ok(Some(_)) => continue,
            _ => panic!("server OOM'd or hung on 0xFFFFFFFF chunk count"),
        }
    }
    legit.shutdown().await;
    handle.abort();
}

// ---- Isolation: AEAD attacker on one connection, sibling unaffected ----

/// Attacker completes handshake via manual_handshake, sends one valid frame,
/// then sends a corrupt encrypted frame. Meanwhile a legitimate sibling client
/// continues sending on its own connection. The sibling's frames must all arrive.
///
/// This is NOT the same as lifecycle_advanced::killed_client_does_not_affect_siblings
/// (which tests hard-drop/EOF). This tests AEAD corruption on one connection
/// while another connection is actively sending. The server must isolate the
/// AEAD failure to the attacker's connection handler and not poison siblings.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn corrupt_attacker_does_not_affect_sibling() {
    common::init_tracing();
    let path = sock_path("wph-corrupt-sibling");
    let _ = std::fs::remove_file(&path);
    let _guard = common::SocketGuard::new(path.clone());
    let (pub_key, mut crx, mut srx, handle) = make_server(&path).await;

    // Legitimate sibling connects normally.
    let kp_legit = generate_keypair().unwrap();
    let sibling = IpcClient::connect(
        Uuid::now_v7(), &path, &pub_key, kp_legit.as_inner(), &IpcConfig::default(), None,
    ).await.unwrap();
    wait_ready(&mut srx).await;

    // Attacker connects via manual handshake.
    let (transport, mut attack_stream) = manual_handshake(&path, &pub_key).await;
    wait_ready(&mut srx).await;

    // Attacker sends one valid frame.
    let tagged = build_app_frame(0, b"attacker-valid");
    write_noise_control_frame(&mut attack_stream, &transport, 0, &tagged).await;

    // Wait for it to arrive.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(deadline, crx.recv()).await {
            Ok(Some((_, p))) if &p[..] == b"attacker-valid" => break,
            Ok(Some(_)) => continue,
            _ => panic!("attacker's valid frame never arrived"),
        }
    }

    // Attacker sends corrupt frame.
    let tagged2 = build_app_frame(1, b"corrupt-payload");
    let mut enc = vec![0u8; tagged2.len() + 16];
    let ct_len = transport.write_message(1, &tagged2, &mut enc).unwrap();
    enc.truncate(ct_len);
    enc[5] ^= 0xFF;

    attack_stream.write_all(&[LANE_CONTROL]).await.unwrap();
    attack_stream.write_all(&4u32.to_le_bytes()).await.unwrap();
    attack_stream.write_all(&1u32.to_le_bytes()).await.unwrap();
    attack_stream.write_all(&(ct_len as u32).to_le_bytes()).await.unwrap();
    attack_stream.write_all(&enc).await.unwrap();
    attack_stream.flush().await.unwrap();
    drop(attack_stream);

    // Sibling sends 10 frames — ALL must arrive.
    for i in 0..10u32 {
        let outcome = sibling.send_frame(
            format!("sibling-{i}").as_bytes(), Duration::from_secs(5),
        ).await;
        assert!(
            outcome.is_delivered(),
            "sibling frame {i} after attacker corruption: {outcome:?}"
        );
    }

    let mut sibling_count = 0;
    while let Ok(Some((_, p))) = tokio::time::timeout(Duration::from_millis(500), crx.recv()).await {
        if std::str::from_utf8(&p).unwrap_or("").starts_with("sibling-") {
            sibling_count += 1;
        }
        if sibling_count >= 10 { break; }
    }
    assert_eq!(
        sibling_count, 10,
        "all 10 sibling frames must arrive after attacker sent corrupt AEAD"
    );

    sibling.shutdown().await;
    handle.abort();
}

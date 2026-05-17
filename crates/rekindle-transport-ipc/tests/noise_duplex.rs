//! Noise handshake tests over tokio::io::duplex — no socket, pure crypto.
//! Proves encrypt/decrypt pair works, nonces sync, persistent buffers
//! don't leak stale data, prologue mismatch fails.

mod common;

use rekindle_transport_ipc::noise::keys::generate_keypair;
use rekindle_transport_ipc::noise::{client_handshake, server_handshake};
use rekindle_transport_ipc::socket::PeerCredentials;

#[tokio::test]
async fn handshake_and_roundtrip() {
    common::init_tracing();
    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let sc = PeerCredentials { pid: 1, uid: 1000 };
    let cc = PeerCredentials { pid: 2, uid: 1000 };

    let (cs, ss) = tokio::io::duplex(65536);
    let (mut cr, mut cw) = tokio::io::split(cs);
    let (mut sr, mut sw) = tokio::io::split(ss);

    let (client_result, server_result) = tokio::join!(
        client_handshake(&mut cr, &mut cw, &server_pub, client_kp.as_inner(), &cc, &sc, std::time::Duration::from_secs(5), rekindle_transport_ipc::frame::codec::MAX_FRAME_SIZE),
        server_handshake(&mut sr, &mut sw, server_kp.as_inner(), &sc, &cc, std::time::Duration::from_secs(5), rekindle_transport_ipc::frame::codec::MAX_FRAME_SIZE),
    );

    let mut ct = client_result.unwrap();
    let mut st = server_result.unwrap();

    // Client -> Server
    ct.writer.write_encrypted_frame(&mut cw, b"hello").await.unwrap();
    tokio::io::AsyncWriteExt::flush(&mut cw).await.unwrap();
    let decrypted = st.reader.read_encrypted_frame(&mut sr).await.unwrap();
    assert_eq!(&decrypted[..], b"hello");

    // Server -> Client
    st.writer.write_encrypted_frame(&mut sw, b"world").await.unwrap();
    tokio::io::AsyncWriteExt::flush(&mut sw).await.unwrap();
    let decrypted = ct.reader.read_encrypted_frame(&mut cr).await.unwrap();
    assert_eq!(&decrypted[..], b"world");
}

#[tokio::test]
async fn both_sides_derive_same_handshake_hash() {
    common::init_tracing();
    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let sc = PeerCredentials { pid: 1, uid: 1000 };
    let cc = PeerCredentials { pid: 2, uid: 1000 };

    let (cs, ss) = tokio::io::duplex(65536);
    let (mut cr, mut cw) = tokio::io::split(cs);
    let (mut sr, mut sw) = tokio::io::split(ss);

    let (mut ct, mut st) = tokio::join!(
        async { client_handshake(&mut cr, &mut cw, &server_pub, client_kp.as_inner(), &cc, &sc, std::time::Duration::from_secs(5), rekindle_transport_ipc::frame::codec::MAX_FRAME_SIZE).await.unwrap() },
        async { server_handshake(&mut sr, &mut sw, server_kp.as_inner(), &sc, &cc, std::time::Duration::from_secs(5), rekindle_transport_ipc::frame::codec::MAX_FRAME_SIZE).await.unwrap() },
    );

    let ch = ct.take_handshake_hash();
    let sh = st.take_handshake_hash();
    assert!(ch.is_some());
    assert!(sh.is_some());
    assert_eq!(ch.unwrap(), sh.unwrap());
    assert!(ct.take_handshake_hash().is_none());
}

#[tokio::test]
async fn nonces_stay_synchronized() {
    common::init_tracing();
    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let sc = PeerCredentials { pid: 1, uid: 1000 };
    let cc = PeerCredentials { pid: 2, uid: 1000 };

    let (cs, ss) = tokio::io::duplex(256 * 1024);
    let (cr, cw) = tokio::io::split(cs);
    let (sr, sw) = tokio::io::split(ss);
    let mut cr = tokio::io::BufReader::new(cr);
    let mut cw = tokio::io::BufWriter::new(cw);
    let mut sr = tokio::io::BufReader::new(sr);
    let mut sw = tokio::io::BufWriter::new(sw);

    let (mut ct, mut st) = tokio::join!(
        async { client_handshake(&mut cr, &mut cw, &server_pub, client_kp.as_inner(), &cc, &sc, std::time::Duration::from_secs(5), rekindle_transport_ipc::frame::codec::MAX_FRAME_SIZE).await.unwrap() },
        async { server_handshake(&mut sr, &mut sw, server_kp.as_inner(), &sc, &cc, std::time::Duration::from_secs(5), rekindle_transport_ipc::frame::codec::MAX_FRAME_SIZE).await.unwrap() },
    );

    for i in 0u64..20 {
        assert_eq!(ct.writer.send_nonce(), st.reader.recv_nonce(), "desync before frame {i}");
        let payload = format!("frame {i}").into_bytes();
        ct.writer.write_encrypted_frame(&mut cw, &payload).await.unwrap();
        tokio::io::AsyncWriteExt::flush(&mut cw).await.unwrap();
        let decrypted = st.reader.read_encrypted_frame(&mut sr).await.unwrap();
        assert_eq!(&decrypted[..], &payload[..]);
        assert_eq!(ct.writer.send_nonce(), st.reader.recv_nonce(), "desync after frame {i}");
    }
}

#[tokio::test]
async fn varying_sizes_no_stale_data() {
    common::init_tracing();
    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let sc = PeerCredentials { pid: 1, uid: 1000 };
    let cc = PeerCredentials { pid: 2, uid: 1000 };

    let (cs, ss) = tokio::io::duplex(256 * 1024);
    let (cr, cw) = tokio::io::split(cs);
    let (sr, sw) = tokio::io::split(ss);
    let mut cr = tokio::io::BufReader::new(cr);
    let mut cw = tokio::io::BufWriter::new(cw);
    let mut sr = tokio::io::BufReader::new(sr);
    let mut sw = tokio::io::BufWriter::new(sw);

    let (mut ct, mut st) = tokio::join!(
        async { client_handshake(&mut cr, &mut cw, &server_pub, client_kp.as_inner(), &cc, &sc, std::time::Duration::from_secs(5), rekindle_transport_ipc::frame::codec::MAX_FRAME_SIZE).await.unwrap() },
        async { server_handshake(&mut sr, &mut sw, server_kp.as_inner(), &sc, &cc, std::time::Duration::from_secs(5), rekindle_transport_ipc::frame::codec::MAX_FRAME_SIZE).await.unwrap() },
    );

    let sizes = [1, 16, 64, 255, 1024, 4096, 8192, 32768, 65519];
    for (i, &size) in sizes.iter().enumerate() {
        let fill = (i as u8).wrapping_add(1);
        let payload = vec![fill; size];
        ct.writer.write_encrypted_frame(&mut cw, &payload).await.unwrap();
        tokio::io::AsyncWriteExt::flush(&mut cw).await.unwrap();
        let decrypted = st.reader.read_encrypted_frame(&mut sr).await.unwrap();
        assert_eq!(decrypted.len(), size, "wrong length at frame {i}");
        assert!(decrypted.iter().all(|&b| b == fill), "stale data at frame {i}");
    }
}

#[tokio::test]
async fn prologue_mismatch_fails() {
    common::init_tracing();
    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let sc = PeerCredentials { pid: 1, uid: 1000 };
    let cc_real = PeerCredentials { pid: 2, uid: 1000 };
    let cc_fake = PeerCredentials { pid: 99, uid: 9999 };

    let (cs, ss) = tokio::io::duplex(65536);
    let (mut cr, mut cw) = tokio::io::split(cs);
    let (mut sr, mut sw) = tokio::io::split(ss);

    let (client_result, server_result) = tokio::join!(
        client_handshake(&mut cr, &mut cw, &server_pub, client_kp.as_inner(), &cc_real, &sc, std::time::Duration::from_secs(5), rekindle_transport_ipc::frame::codec::MAX_FRAME_SIZE),
        server_handshake(&mut sr, &mut sw, server_kp.as_inner(), &sc, &cc_fake, std::time::Duration::from_secs(5), rekindle_transport_ipc::frame::codec::MAX_FRAME_SIZE),
    );

    assert!(
        client_result.is_err() || server_result.is_err(),
        "prologue mismatch must fail handshake"
    );
}

/// Upstream reference: snow/tests/general.rs:925-957 test_stateful_nonce_increment_behavior
///
/// Proves: when NoiseReader decryption fails on corrupted ciphertext, the
/// recv_nonce counter is NOT advanced. The receiver can retry with the
/// correct ciphertext at the same nonce and succeed. After the successful
/// decrypt, the nonce IS advanced, so a third attempt with the same
/// ciphertext fails.
///
/// This catches the bug where NoiseReader does:
///   let nonce = self.recv_nonce.fetch_add(1, ...); // WRONG: advances before decrypt
///   self.state.read_message(nonce, ...)
/// instead of:
///   let nonce = self.recv_nonce.load(...);
///   self.state.read_message(nonce, ...)?;
///   self.recv_nonce.fetch_add(1, ...); // only advance on success
///
/// WILL FAIL if NoiseReader (reader.rs:67) uses fetch_add before decrypt.
#[tokio::test]
async fn nonce_not_consumed_on_decrypt_failure() {
    common::init_tracing();
    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let sc = PeerCredentials { pid: 1, uid: 1000 };
    let cc = PeerCredentials { pid: 2, uid: 1000 };

    let (cs, ss) = tokio::io::duplex(256 * 1024);
    let (cr, cw) = tokio::io::split(cs);
    let (sr, sw) = tokio::io::split(ss);
    let mut cr = tokio::io::BufReader::new(cr);
    let mut cw = tokio::io::BufWriter::new(cw);
    let mut sr = tokio::io::BufReader::new(sr);
    let mut sw = tokio::io::BufWriter::new(sw);

    let (mut ct, mut st) = tokio::join!(
        async { client_handshake(&mut cr, &mut cw, &server_pub, client_kp.as_inner(), &cc, &sc, std::time::Duration::from_secs(5), rekindle_transport_ipc::frame::codec::MAX_FRAME_SIZE).await.unwrap() },
        async { server_handshake(&mut sr, &mut sw, server_kp.as_inner(), &sc, &cc, std::time::Duration::from_secs(5), rekindle_transport_ipc::frame::codec::MAX_FRAME_SIZE).await.unwrap() },
    );

    // Phase 1: valid frame, establish baseline.
    let payload = b"nonce-test-valid";
    ct.writer.write_encrypted_frame(&mut cw, payload).await.unwrap();
    tokio::io::AsyncWriteExt::flush(&mut cw).await.unwrap();
    let decrypted = st.reader.read_encrypted_frame(&mut sr).await.unwrap();
    assert_eq!(&decrypted[..], payload, "baseline roundtrip failed");

    // Phase 2: capture raw encrypted bytes for a second frame, then corrupt them.
    // We write to a capture buffer instead of the real stream.
    let (cap_c, cap_s) = tokio::io::duplex(256 * 1024);
    let (mut cap_cr, _) = tokio::io::split(cap_s);
    let (_, cap_cw) = tokio::io::split(cap_c);
    let mut cap_cw = tokio::io::BufWriter::new(cap_cw);

    let payload2 = b"will-be-corrupted";
    ct.writer.write_encrypted_frame(&mut cap_cw, payload2).await.unwrap();
    tokio::io::AsyncWriteExt::flush(&mut cap_cw).await.unwrap();
    drop(cap_cw); // close write end so read sees all bytes

    // Read the raw encrypted frame bytes from the capture.
    let mut raw_frame = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut cap_cr, &mut raw_frame).await.unwrap();
    assert!(!raw_frame.is_empty(), "captured frame is empty");

    // Keep uncorrupted copy for retry.
    let uncorrupted = raw_frame.clone();

    // Corrupt one byte deep in the ciphertext (past the length prefixes).
    // The frame layout is: [4B chunk_count_len][4B chunk_count][4B chunk_len][encrypted_chunk]
    // We corrupt inside the encrypted chunk.
    let corrupt_offset = raw_frame.len() - 5;
    raw_frame[corrupt_offset] ^= 0xFF;

    // Send the corrupted frame to the server reader.
    tokio::io::AsyncWriteExt::write_all(&mut cw, &raw_frame).await.unwrap();
    tokio::io::AsyncWriteExt::flush(&mut cw).await.unwrap();

    // Server reader should fail.
    let result = st.reader.read_encrypted_frame(&mut sr).await;
    assert!(result.is_err(), "corrupted ciphertext must fail decryption");

    // Phase 3: send the UNCORRUPTED frame. If the nonce was NOT consumed
    // on the failed decrypt, this succeeds because the server still expects
    // the same nonce.
    tokio::io::AsyncWriteExt::write_all(&mut cw, &uncorrupted).await.unwrap();
    tokio::io::AsyncWriteExt::flush(&mut cw).await.unwrap();

    let retry_result = st.reader.read_encrypted_frame(&mut sr).await;
    assert!(
        retry_result.is_ok(),
        "retry with uncorrupted ciphertext must succeed — \
         nonce must NOT advance on decrypt failure. \
         If this fails, NoiseReader::read_encrypted_frame advances \
         recv_nonce before attempting decrypt (reader.rs:67)."
    );
    assert_eq!(&retry_result.unwrap()[..], payload2);

    // Phase 4: replay same ciphertext. Now the nonce WAS consumed on the
    // successful decrypt, so the server expects nonce+1. This must fail.
    tokio::io::AsyncWriteExt::write_all(&mut cw, &uncorrupted).await.unwrap();
    tokio::io::AsyncWriteExt::flush(&mut cw).await.unwrap();

    let replay_result = st.reader.read_encrypted_frame(&mut sr).await;
    assert!(
        replay_result.is_err(),
        "replaying same ciphertext after successful decrypt must fail — \
         nonce must advance on success"
    );
}

/// Proves: directional bulk keys are distinct and cross-direction
/// decrypt fails. Client's send key (initiator_send) must NOT decrypt
/// data encrypted with server's send key (responder_send), and vice versa.
/// If both sides used the same key, this test would pass — proving nonce
/// reuse is possible. This test MUST fail with a shared key.
///
/// Uses two derivations from the same handshake hash (deterministic) to
/// get independent BulkCipher instances for the same key material.
#[test]
fn directional_bulk_keys_prevent_cross_decrypt() {
    use rekindle_transport_ipc::bulk::kdf::derive_bulk_key_pair;

    let handshake_hash = [0x42u8; 32];
    // Derive two pairs from the same hash — deterministic, same keys.
    // pair1 for encrypt, pair2 for decrypt verification (BulkCipher is !Clone).
    let pair1 = derive_bulk_key_pair(&handshake_hash);
    let pair2 = derive_bulk_key_pair(&handshake_hash);

    // Client encrypts with initiator_send at nonce 0.
    let mut buf = b"client-to-server-secret".to_vec();
    let tag = pair1.initiator_send.seal_in_place(0, b"", &mut buf).unwrap();

    // Server decrypts with initiator_send (correct direction) — must succeed.
    let mut combined = Vec::new();
    combined.extend_from_slice(&buf);
    combined.extend_from_slice(&tag);
    assert!(pair2.initiator_send.open_in_place(0, b"", &mut combined).is_ok(),
        "correct-direction decrypt must succeed");

    // Server decrypts with responder_send (WRONG direction) — must fail.
    let mut combined_wrong = Vec::new();
    combined_wrong.extend_from_slice(&buf);
    combined_wrong.extend_from_slice(&tag);
    assert!(pair2.responder_send.open_in_place(0, b"", &mut combined_wrong).is_err(),
        "cross-direction decrypt must fail — keys must be different");

    // Verify nonce 0 on both directions uses different keys → different ciphertext.
    let mut buf_server = b"server-to-client-secret".to_vec();
    let tag_server = pair1.responder_send.seal_in_place(0, b"", &mut buf_server).unwrap();
    // Same nonce (0), different keys → different ciphertext.
    assert_ne!(buf, buf_server, "same nonce on different keys must produce different ciphertext");
    assert_ne!(tag, tag_server, "same nonce on different keys must produce different tags");
}

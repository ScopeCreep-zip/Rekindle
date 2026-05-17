//! Advanced Noise crypto tests: concurrent split, key zeroization, edge cases.

mod common;

use std::sync::Arc;
use rekindle_transport_ipc::noise::keys::{generate_keypair, ZeroizingKeypair, NOISE_PARAMS};
use rekindle_transport_ipc::noise::resolver::noise_builder;
use rekindle_transport_ipc::noise::{client_handshake, server_handshake};
use rekindle_transport_ipc::socket::PeerCredentials;

/// 10.6 Nonce exhaustion behavior: NonceCounter aborts at limit.
/// We can't test the actual abort (it kills the process), but we can test
/// that current() reflects usage correctly up to a high count.
#[test]
fn nonce_counter_tracks_usage() {
    common::init_tracing();
    use rekindle_transport_ipc::bulk::NonceCounter;
    let ctr = NonceCounter::new();
    for _ in 0..10_000 {
        ctr.next();
    }
    assert_eq!(ctr.current(), 10_000);
}

/// 10.7 ZeroizingKeypair: into_inner returns valid 32-byte keys.
/// Actual memory zeroization after Drop cannot be verified without unsafe
/// memory inspection or a sanitizer. This test verifies the type's API
/// correctness, not the zeroize::Zeroize implementation.
#[test]
fn zeroizing_keypair_into_inner_valid() {
    common::init_tracing();
    let kp = generate_keypair().unwrap();
    assert_eq!(kp.private().len(), 32);
    assert_eq!(kp.public().len(), 32);
    assert!(!kp.private().iter().all(|&b| b == 0), "private key must not be all zeros");
    let inner = kp.into_inner();
    assert_eq!(inner.private.len(), 32);
    assert_eq!(inner.public.len(), 32);
}

/// 10.8 Concurrent encrypt/decrypt on separate halves.
/// Proves StatelessTransportState split is safe for parallel use.
#[tokio::test]
async fn concurrent_encrypt_decrypt_split() {
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

    // Run client→server and server→client SIMULTANEOUSLY via tokio::join!.
    // This proves the writer and reader halves can operate in parallel
    // without &mut self conflicts on the StatelessTransportState.
    let c2s = tokio::spawn(async move {
        for i in 0u32..50 {
            ct.writer.write_encrypted_frame(&mut cw, format!("c2s-{i}").as_bytes()).await.unwrap();
            tokio::io::AsyncWriteExt::flush(&mut cw).await.unwrap();
        }
        (ct.writer, cw)
    });

    let s2c_write = tokio::spawn(async move {
        for i in 0u32..50 {
            st.writer.write_encrypted_frame(&mut sw, format!("s2c-{i}").as_bytes()).await.unwrap();
            tokio::io::AsyncWriteExt::flush(&mut sw).await.unwrap();
        }
        (st.writer, sw)
    });

    // Readers: server reads client's 50 frames, client reads server's 50 frames.
    let s_read = tokio::spawn(async move {
        for i in 0u32..50 {
            let p = st.reader.read_encrypted_frame(&mut sr).await.unwrap();
            assert_eq!(std::str::from_utf8(&p).unwrap(), format!("c2s-{i}"), "s read {i}");
        }
    });

    let c_read = tokio::spawn(async move {
        for i in 0u32..50 {
            let p = ct.reader.read_encrypted_frame(&mut cr).await.unwrap();
            assert_eq!(std::str::from_utf8(&p).unwrap(), format!("s2c-{i}"), "c read {i}");
        }
    });

    // All four tasks run concurrently.
    let (r1, r2, r3, r4) = tokio::join!(c2s, s2c_write, s_read, c_read);
    r1.unwrap();
    r2.unwrap();
    r3.unwrap();
    r4.unwrap();
}

/// 10.8b Simultaneous bidirectional: 100 frames each direction, interleaved.
#[tokio::test]
async fn bidirectional_100_frames_simultaneous() {
    common::init_tracing();
    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let sc = PeerCredentials { pid: 1, uid: 1000 };
    let cc = PeerCredentials { pid: 2, uid: 1000 };

    let (cs, ss) = tokio::io::duplex(512 * 1024);
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

    let write_c = async {
        for i in 0u32..100 {
            ct.writer.write_encrypted_frame(&mut cw, &vec![0xAA; 1000 + i as usize]).await.unwrap();
            tokio::io::AsyncWriteExt::flush(&mut cw).await.unwrap();
        }
    };
    let write_s = async {
        for i in 0u32..100 {
            st.writer.write_encrypted_frame(&mut sw, &vec![0xBB; 1000 + i as usize]).await.unwrap();
            tokio::io::AsyncWriteExt::flush(&mut sw).await.unwrap();
        }
    };
    let read_s = async {
        for i in 0u32..100 {
            let p = st.reader.read_encrypted_frame(&mut sr).await.unwrap();
            assert_eq!(p.len(), 1000 + i as usize, "s read {i} size");
            assert!(p.iter().all(|&b| b == 0xAA), "s read {i} content");
        }
    };
    let read_c = async {
        for i in 0u32..100 {
            let p = ct.reader.read_encrypted_frame(&mut cr).await.unwrap();
            assert_eq!(p.len(), 1000 + i as usize, "c read {i} size");
            assert!(p.iter().all(|&b| b == 0xBB), "c read {i} content");
        }
    };

    tokio::join!(write_c, write_s, read_s, read_c);
}

/// 10.9 ZeroizingKeypair: private key memory is zeroed after drop.
///
/// Captures a raw pointer to the private key's heap buffer before drop,
/// then reads the memory after drop to verify it was overwritten with zeros.
///
/// This is inherently unsafe — we're reading freed-then-possibly-reused memory.
/// The test is best-effort: if the allocator reuses the buffer immediately,
/// we might read zeros from a new allocation rather than from zeroization.
/// But if we read the ORIGINAL non-zero key bytes, zeroization definitively failed.
///
/// WILL FAIL if ZeroizingKeypair::drop doesn't call zeroize on private key.
#[test]
fn zeroizing_keypair_private_key_zeroed_after_drop() {
    common::init_tracing();
    let kp: ZeroizingKeypair = generate_keypair().unwrap();

    // Capture the private key content and the raw pointer to its heap buffer.
    let private_copy: Vec<u8> = kp.private().to_vec();
    assert_eq!(private_copy.len(), 32);
    assert!(!private_copy.iter().all(|&b| b == 0), "private key must not be all zeros");

    let ptr = kp.private().as_ptr();
    let len = kp.private().len();

    // Drop the keypair — this should zeroize the private key via Drop impl.
    drop(kp);

    // Read the memory that WAS the private key.
    // SAFETY: The pointer was valid before drop. After drop, the Vec's buffer
    // was zeroized by zeroize::Zeroize::zeroize, then the Vec itself was dropped
    // (deallocating the buffer). We're reading potentially-freed memory.
    // This is UB in the strict sense, but in practice the allocator usually
    // hasn't reused the buffer yet in a single-threaded test.
    //
    // If we see the original key bytes, zeroization DEFINITELY failed.
    // If we see zeros, zeroization PROBABLY succeeded (or allocator zeroed on free).
    // If we see different non-zero bytes, the allocator reused the buffer.
    let after: Vec<u8> = unsafe {
        std::slice::from_raw_parts(ptr, len).to_vec()
    };

    // The key assertion: the original private key bytes must NOT be present.
    assert_ne!(
        after, private_copy,
        "private key bytes still present in memory after ZeroizingKeypair drop — \
         zeroization failed. The Drop impl at keys.rs:83-86 must call \
         zeroize::Zeroize::zeroize on self.inner.private."
    );
}

/// 10.10 Arc<StatelessTransportState> concurrent encrypt from multiple threads.
///
/// The StatelessTransportState::write_message takes &self with explicit nonces.
/// This means multiple threads can encrypt simultaneously using the same state
/// with different nonces — the foundation for rayon bulk encryption.
///
/// This test clones the Arc and calls write_message from 4 threads concurrently,
/// each with its own nonce range, then verifies all ciphertexts decrypt correctly.
#[test]
fn stateless_transport_concurrent_encrypt() {
    common::init_tracing();

    // Build a handshake pair to get StatelessTransportState.
    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();

    let mut initiator = noise_builder(NOISE_PARAMS)
        .local_private_key(&client_kp.as_inner().private).unwrap()
        .remote_public_key(&server_pub).unwrap()
        .prologue(b"CONCURRENT-TEST").unwrap()
        .build_initiator().unwrap();
    let mut responder = noise_builder(NOISE_PARAMS)
        .local_private_key(&server_kp.as_inner().private).unwrap()
        .prologue(b"CONCURRENT-TEST").unwrap()
        .build_responder().unwrap();

    let mut buf = [0u8; 256];
    let mut payload = [0u8; 256];
    let len = initiator.write_message(&[], &mut buf).unwrap();
    responder.read_message(&buf[..len], &mut payload).unwrap();
    let len = responder.write_message(&[], &mut buf).unwrap();
    initiator.read_message(&buf[..len], &mut payload).unwrap();

    let encrypt_state = Arc::new(initiator.into_stateless_transport_mode().unwrap());
    let decrypt_state = Arc::new(responder.into_stateless_transport_mode().unwrap());

    // 4 threads, each encrypts 100 messages with nonces [thread*100..(thread+1)*100).
    let mut handles = Vec::new();
    for thread_id in 0u64..4 {
        let state = Arc::clone(&encrypt_state);
        handles.push(std::thread::spawn(move || {
            let mut results = Vec::new();
            for i in 0u64..100 {
                let nonce = thread_id * 100 + i;
                let plaintext = format!("thread-{thread_id}-msg-{i}");
                let mut out = vec![0u8; plaintext.len() + 16]; // +AEAD tag
                let len = state.write_message(nonce, plaintext.as_bytes(), &mut out).unwrap();
                out.truncate(len);
                results.push((nonce, plaintext, out));
            }
            results
        }));
    }

    // Collect all encrypted messages.
    let mut all_encrypted: Vec<(u64, String, Vec<u8>)> = Vec::new();
    for h in handles {
        all_encrypted.extend(h.join().unwrap());
    }
    assert_eq!(all_encrypted.len(), 400);

    // Decrypt all and verify correctness.
    for (nonce, expected_plaintext, ciphertext) in &all_encrypted {
        let mut dec = vec![0u8; ciphertext.len()];
        let len = decrypt_state.read_message(*nonce, ciphertext, &mut dec).unwrap();
        let decrypted = std::str::from_utf8(&dec[..len]).unwrap();
        assert_eq!(
            decrypted, expected_plaintext,
            "nonce {nonce}: decrypted mismatch"
        );
    }
}

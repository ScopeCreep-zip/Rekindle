//! Tests for the Noise IK transport module.

use super::*;
use crate::ipc::noise_keys::generate_keypair;
use crate::ipc::transport::PeerCredentials;

#[test]
fn prologue_canonical_ordering() {
    let a = PeerCredentials { pid: 100, uid: 1000 };
    let b = PeerCredentials { pid: 200, uid: 1000 };
    assert_eq!(build_prologue(&a, &b), build_prologue(&b, &a));
    let p = String::from_utf8(build_prologue(&a, &b)).unwrap();
    assert_eq!(p, "REKINDLE-IPC-v1:100:1000:200:1000");
}

#[tokio::test]
async fn handshake_and_transport_roundtrip() {
    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();

    let sc = PeerCredentials { pid: 1, uid: 1000 };
    let cc = PeerCredentials { pid: 2, uid: 1000 };

    let (cs, ss) = tokio::io::duplex(65536);
    let (mut cr, mut cw) = tokio::io::split(cs);
    let (mut sr, mut sw) = tokio::io::split(ss);

    let (client_result, server_result) = tokio::join!(
        client_handshake(&mut cr, &mut cw, &server_pub, client_kp.as_inner(), &cc, &sc),
        server_handshake(&mut sr, &mut sw, server_kp.as_inner(), &sc, &cc),
    );

    let mut ct = client_result.unwrap();
    let mut st = server_result.unwrap();

    let plaintext = b"hello encrypted world";
    ct.writer.write_encrypted_frame(&mut cw, plaintext).await.unwrap();
    let decrypted = st.reader.read_encrypted_frame(&mut sr).await.unwrap();
    assert_eq!(&decrypted[..], &plaintext[..]);

    let response = b"acknowledged";
    st.writer.write_encrypted_frame(&mut sw, response).await.unwrap();
    let decrypted_response = ct.reader.read_encrypted_frame(&mut cr).await.unwrap();
    assert_eq!(&decrypted_response[..], &response[..]);
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
        async { client_handshake(&mut cr, &mut cw, &server_pub, client_kp.as_inner(), &cc, &sc).await.unwrap() },
        async { server_handshake(&mut sr, &mut sw, server_kp.as_inner(), &sc, &cc).await.unwrap() },
    );

    let large_payload = vec![0xABu8; 200 * 1024];
    ct.writer.write_encrypted_frame(&mut cw, &large_payload).await.unwrap();
    let decrypted = st.reader.read_encrypted_frame(&mut sr).await.unwrap();
    assert_eq!(&decrypted[..], &large_payload[..]);
}

#[tokio::test]
async fn prologue_mismatch_fails_handshake() {
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
        client_handshake(&mut cr, &mut cw, &server_pub, client_kp.as_inner(), &cc_real, &sc),
        server_handshake(&mut sr, &mut sw, server_kp.as_inner(), &sc, &cc_fake),
    );

    assert!(
        client_result.is_err() || server_result.is_err(),
        "prologue mismatch must cause handshake failure"
    );
}

#[tokio::test]
async fn empty_payload_roundtrip() {
    let (mut ct, mut st, mut cw, mut sr) = make_buffered_pair().await;

    ct.writer.write_encrypted_frame(&mut cw, b"").await.unwrap();
    use tokio::io::AsyncWriteExt;
    cw.flush().await.unwrap();
    let decrypted = st.reader.read_encrypted_frame(&mut sr).await.unwrap();
    assert!(decrypted.is_empty());
}

async fn make_buffered_pair() -> (
    NoiseTransport,
    NoiseTransport,
    tokio::io::BufWriter<tokio::io::WriteHalf<tokio::io::DuplexStream>>,
    tokio::io::BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
) {
    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();

    let sc = PeerCredentials { pid: 1, uid: 1000 };
    let cc = PeerCredentials { pid: 2, uid: 1000 };

    let (cs, ss) = tokio::io::duplex(256 * 1024);
    let (cr, cw) = tokio::io::split(cs);
    let (sr, sw) = tokio::io::split(ss);

    let mut cr = tokio::io::BufReader::with_capacity(8192, cr);
    let mut cw = tokio::io::BufWriter::with_capacity(8192, cw);
    let mut sr = tokio::io::BufReader::with_capacity(8192, sr);
    let mut sw = tokio::io::BufWriter::with_capacity(8192, sw);

    let (ct, st) = tokio::join!(
        client_handshake(&mut cr, &mut cw, &server_pub, client_kp.as_inner(), &cc, &sc),
        server_handshake(&mut sr, &mut sw, server_kp.as_inner(), &sc, &cc),
    );

    (ct.unwrap(), st.unwrap(), cw, sr)
}

#[tokio::test]
async fn max_size_frame_roundtrip() {
    use tokio::io::AsyncWriteExt;
    let (mut ct, mut st, mut writer, mut reader) = make_buffered_pair().await;

    let max_plaintext = vec![0x42u8; MAX_NOISE_PLAINTEXT];
    ct.writer.write_encrypted_frame(&mut writer, &max_plaintext).await.unwrap();
    writer.flush().await.unwrap();

    let decrypted = st.reader.read_encrypted_frame(&mut reader).await.unwrap();
    assert_eq!(decrypted.len(), MAX_NOISE_PLAINTEXT);
    assert!(decrypted.iter().all(|&b| b == 0x42));
}

#[tokio::test]
async fn nonce_stays_synchronized_across_frames() {
    use tokio::io::AsyncWriteExt;
    let (mut ct, mut st, mut writer, mut reader) = make_buffered_pair().await;

    for i in 0u64..20 {
        assert_eq!(
            ct.writer.sending_nonce(),
            st.reader.receiving_nonce(),
            "nonce desync before frame {i}"
        );

        let payload = format!("frame {i}").into_bytes();
        ct.writer.write_encrypted_frame(&mut writer, &payload).await.unwrap();
        writer.flush().await.unwrap();

        let decrypted = st.reader.read_encrypted_frame(&mut reader).await.unwrap();
        assert_eq!(&decrypted[..], &payload[..], "data mismatch at frame {i}");

        assert_eq!(
            ct.writer.sending_nonce(),
            st.reader.receiving_nonce(),
            "nonce desync after frame {i}"
        );
    }
}

#[tokio::test]
async fn persistent_buffers_no_stale_data() {
    use tokio::io::AsyncWriteExt;
    let (mut ct, mut st, mut writer, mut reader) = make_buffered_pair().await;

    let sizes = [1, 16, 64, 255, 256, 1024, 4096, 8191, 8192, 8193, 32768, MAX_NOISE_PLAINTEXT];

    for (i, &size) in sizes.iter().enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        let fill = (i as u8).wrapping_add(1);
        let payload = vec![fill; size];

        ct.writer.write_encrypted_frame(&mut writer, &payload).await.unwrap();
        writer.flush().await.unwrap();

        let decrypted = st.reader.read_encrypted_frame(&mut reader).await.unwrap();
        assert_eq!(decrypted.len(), size, "wrong length at frame {i} (size={size})");
        assert!(
            decrypted.iter().all(|&b| b == fill),
            "stale data at frame {i} (size={size}, fill=0x{fill:02x})"
        );
    }
}

#[tokio::test]
async fn split_bytes_independent_across_reads() {
    use tokio::io::AsyncWriteExt;
    let (mut ct, mut st, mut writer, mut reader) = make_buffered_pair().await;

    ct.writer.write_encrypted_frame(&mut writer, b"first").await.unwrap();
    writer.flush().await.unwrap();
    let first: bytes::Bytes = st.reader.read_encrypted_frame(&mut reader).await.unwrap();

    ct.writer.write_encrypted_frame(&mut writer, b"second").await.unwrap();
    writer.flush().await.unwrap();
    let second: bytes::Bytes = st.reader.read_encrypted_frame(&mut reader).await.unwrap();

    assert_eq!(&first[..], b"first");
    assert_eq!(&second[..], b"second");
}

#[tokio::test]
async fn batch_exceeds_bufwriter_capacity() {
    use tokio::io::AsyncWriteExt;
    let (mut ct, mut st, mut writer, mut reader) = make_buffered_pair().await;

    let messages: Vec<Vec<u8>> = (0..50u8)
        .map(|i| {
            let mut msg = vec![i; 180];
            msg.extend_from_slice(&i.to_le_bytes());
            msg
        })
        .collect();

    for msg in &messages {
        ct.writer.write_encrypted_frame(&mut writer, msg).await.unwrap();
    }
    writer.flush().await.unwrap();

    for (i, msg) in messages.iter().enumerate() {
        let decrypted = st.reader.read_encrypted_frame(&mut reader).await.unwrap();
        assert_eq!(&decrypted[..], &msg[..], "data mismatch at frame {i}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unix_socket_roundtrip() {
    use tokio::io::AsyncWriteExt;
    use tokio::net::UnixListener;

    let sock_path = std::env::temp_dir().join(format!(
        "rekindle-test-{}-{}.sock",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_file(&sock_path);

    let listener = UnixListener::bind(&sock_path).unwrap();

    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();
    let sc = PeerCredentials { pid: 1, uid: 1000 };
    let cc = PeerCredentials { pid: 2, uid: 1000 };

    let (client_stream, server_stream) = tokio::join!(
        tokio::net::UnixStream::connect(&sock_path),
        async { listener.accept().await.map(|(s, _)| s) },
    );
    let client_stream = client_stream.unwrap();
    let server_stream = server_stream.unwrap();

    let (cr, cw) = client_stream.into_split();
    let (sr, sw) = server_stream.into_split();

    let mut cr = tokio::io::BufReader::with_capacity(8192, cr);
    let mut cw = tokio::io::BufWriter::with_capacity(8192, cw);
    let mut sr = tokio::io::BufReader::with_capacity(8192, sr);
    let mut sw = tokio::io::BufWriter::with_capacity(8192, sw);

    let (ct, st) = tokio::join!(
        client_handshake(&mut cr, &mut cw, &server_pub, client_kp.as_inner(), &cc, &sc),
        server_handshake(&mut sr, &mut sw, server_kp.as_inner(), &sc, &cc),
    );
    let mut ct = ct.unwrap();
    let mut st = st.unwrap();

    let plaintext = b"hello over unix socket";
    ct.writer.write_encrypted_frame(&mut cw, plaintext).await.unwrap();
    cw.flush().await.unwrap();

    let decrypted = st.reader.read_encrypted_frame(&mut sr).await.unwrap();
    assert_eq!(&decrypted[..], &plaintext[..]);

    let response = b"acknowledged over unix socket";
    st.writer.write_encrypted_frame(&mut sw, response).await.unwrap();
    sw.flush().await.unwrap();

    let decrypted = ct.reader.read_encrypted_frame(&mut cr).await.unwrap();
    assert_eq!(&decrypted[..], &response[..]);

    let _ = std::fs::remove_file(&sock_path);
}

#[tokio::test]
async fn handshake_hash_is_available() {
    let (mut ct, mut _st, _cw, _sr) = make_buffered_pair().await;
    let hash = ct.take_handshake_hash();
    assert!(hash.is_some());
    assert_eq!(hash.unwrap().len(), 32);

    let second = ct.take_handshake_hash();
    assert!(second.is_none());
}

// ── AWS-LC resolver tests ─────────────────────────────────────────

#[tokio::test]
async fn aws_lc_resolver_handshake_roundtrip() {
    use crate::ipc::noise::aws_lc_resolver::noise_builder;
    use crate::ipc::noise_keys::NOISE_PARAMS;

    // Generate keypairs using the standard builder (key generation
    // is algorithm-agnostic — only the cipher differs).
    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();

    let mut initiator = noise_builder(NOISE_PARAMS)
        .local_private_key(&client_kp.as_inner().private).unwrap()
        .remote_public_key(&server_pub).unwrap()
        .prologue(b"AWSLC_TEST").unwrap()
        .build_initiator().unwrap();
    let mut responder = noise_builder(NOISE_PARAMS)
        .local_private_key(&server_kp.as_inner().private).unwrap()
        .prologue(b"AWSLC_TEST").unwrap()
        .build_responder().unwrap();

    let mut buf = [0u8; 256];
    let mut payload = [0u8; 256];

    let len = initiator.write_message(&[], &mut buf).unwrap();
    responder.read_message(&buf[..len], &mut payload).unwrap();
    let len = responder.write_message(&[], &mut buf).unwrap();
    initiator.read_message(&buf[..len], &mut payload).unwrap();

    let h_i = initiator.into_stateless_transport_mode().unwrap();
    let h_r = responder.into_stateless_transport_mode().unwrap();

    // Roundtrip: encrypt with initiator, decrypt with responder.
    let msg = b"hello aws-lc resolver - zero alloc decrypt";
    let mut enc = vec![0u8; msg.len() + 16];
    let mut dec = vec![0u8; msg.len()];

    let ct_len = h_i.write_message(0, msg, &mut enc).unwrap();
    let pt_len = h_r.read_message(0, &enc[..ct_len], &mut dec).unwrap();
    assert_eq!(&dec[..pt_len], msg);

    // Reverse direction: encrypt with responder, decrypt with initiator.
    let resp = b"acknowledged - symmetric performance";
    let mut enc2 = vec![0u8; resp.len() + 16];
    let mut dec2 = vec![0u8; resp.len()];

    let ct_len2 = h_r.write_message(1, resp, &mut enc2).unwrap();
    let pt_len2 = h_i.read_message(1, &enc2[..ct_len2], &mut dec2).unwrap();
    assert_eq!(&dec2[..pt_len2], resp);
}

#[tokio::test]
async fn aws_lc_resolver_max_noise_payload() {
    use crate::ipc::noise::aws_lc_resolver::noise_builder;
    use crate::ipc::noise_keys::NOISE_PARAMS;

    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();

    let mut initiator = noise_builder(NOISE_PARAMS)
        .local_private_key(&client_kp.as_inner().private).unwrap()
        .remote_public_key(&server_pub).unwrap()
        .prologue(b"LARGE").unwrap()
        .build_initiator().unwrap();
    let mut responder = noise_builder(NOISE_PARAMS)
        .local_private_key(&server_kp.as_inner().private).unwrap()
        .prologue(b"LARGE").unwrap()
        .build_responder().unwrap();

    let mut buf = [0u8; 256];
    let mut payload = [0u8; 256];
    let len = initiator.write_message(&[], &mut buf).unwrap();
    responder.read_message(&buf[..len], &mut payload).unwrap();
    let len = responder.write_message(&[], &mut buf).unwrap();
    initiator.read_message(&buf[..len], &mut payload).unwrap();

    let h_i = initiator.into_stateless_transport_mode().unwrap();
    let h_r = responder.into_stateless_transport_mode().unwrap();

    // Max Noise payload: 65535 - 16 = 65519 bytes.
    let msg = vec![0xABu8; MAX_NOISE_PLAINTEXT];
    let mut enc = vec![0u8; msg.len() + 16];
    let mut dec = vec![0u8; msg.len()];

    let ct_len = h_i.write_message(0, &msg, &mut enc).unwrap();
    let pt_len = h_r.read_message(0, &enc[..ct_len], &mut dec).unwrap();
    assert_eq!(pt_len, msg.len());
    assert_eq!(&dec[..pt_len], &msg[..]);
}

#[tokio::test]
async fn aws_lc_resolver_nonce_sync_across_frames() {
    // Verify that nonce counters stay synchronized when using
    // the aws-lc resolver across multiple encrypt/decrypt calls.
    use crate::ipc::noise::aws_lc_resolver::noise_builder;
    use crate::ipc::noise_keys::NOISE_PARAMS;

    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();

    let mut initiator = noise_builder(NOISE_PARAMS)
        .local_private_key(&client_kp.as_inner().private).unwrap()
        .remote_public_key(&server_pub).unwrap()
        .prologue(b"SYNC").unwrap()
        .build_initiator().unwrap();
    let mut responder = noise_builder(NOISE_PARAMS)
        .local_private_key(&server_kp.as_inner().private).unwrap()
        .prologue(b"SYNC").unwrap()
        .build_responder().unwrap();

    let mut buf = [0u8; 256];
    let mut payload = [0u8; 256];
    let len = initiator.write_message(&[], &mut buf).unwrap();
    responder.read_message(&buf[..len], &mut payload).unwrap();
    let len = responder.write_message(&[], &mut buf).unwrap();
    initiator.read_message(&buf[..len], &mut payload).unwrap();

    let h_i = initiator.into_stateless_transport_mode().unwrap();
    let h_r = responder.into_stateless_transport_mode().unwrap();

    // Send 20 frames with incrementing nonces.
    for nonce in 0u64..20 {
        let msg = format!("frame {nonce}").into_bytes();
        let mut enc = vec![0u8; msg.len() + 16];
        let mut dec = vec![0u8; msg.len()];

        let ct_len = h_i.write_message(nonce, &msg, &mut enc).unwrap();
        let pt_len = h_r.read_message(nonce, &enc[..ct_len], &mut dec).unwrap();
        assert_eq!(&dec[..pt_len], &msg[..], "mismatch at nonce {nonce}");
    }
}

#[tokio::test]
async fn aws_lc_resolver_tampered_ciphertext_rejected() {
    use crate::ipc::noise::aws_lc_resolver::noise_builder;
    use crate::ipc::noise_keys::NOISE_PARAMS;

    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();

    let mut initiator = noise_builder(NOISE_PARAMS)
        .local_private_key(&client_kp.as_inner().private).unwrap()
        .remote_public_key(&server_pub).unwrap()
        .prologue(b"TAMPER").unwrap()
        .build_initiator().unwrap();
    let mut responder = noise_builder(NOISE_PARAMS)
        .local_private_key(&server_kp.as_inner().private).unwrap()
        .prologue(b"TAMPER").unwrap()
        .build_responder().unwrap();

    let mut buf = [0u8; 256];
    let mut payload = [0u8; 256];
    let len = initiator.write_message(&[], &mut buf).unwrap();
    responder.read_message(&buf[..len], &mut payload).unwrap();
    let len = responder.write_message(&[], &mut buf).unwrap();
    initiator.read_message(&buf[..len], &mut payload).unwrap();

    let h_i = initiator.into_stateless_transport_mode().unwrap();
    let h_r = responder.into_stateless_transport_mode().unwrap();

    let msg = b"tamper test payload";
    let mut enc = vec![0u8; msg.len() + 16];
    let ct_len = h_i.write_message(0, msg, &mut enc).unwrap();

    // Tamper with the ciphertext
    enc[0] ^= 0xFF;

    let mut dec = vec![0u8; msg.len()];
    assert!(h_r.read_message(0, &enc[..ct_len], &mut dec).is_err(),
        "tampered ciphertext must be rejected by aws-lc resolver");
}

#[tokio::test]
async fn aws_lc_resolver_short_ciphertext_rejected() {
    use crate::ipc::noise::aws_lc_resolver::noise_builder;
    use crate::ipc::noise_keys::NOISE_PARAMS;

    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let server_pub: [u8; 32] = server_kp.public().try_into().unwrap();

    let mut initiator = noise_builder(NOISE_PARAMS)
        .local_private_key(&client_kp.as_inner().private).unwrap()
        .remote_public_key(&server_pub).unwrap()
        .prologue(b"SHORT").unwrap()
        .build_initiator().unwrap();
    let mut responder = noise_builder(NOISE_PARAMS)
        .local_private_key(&server_kp.as_inner().private).unwrap()
        .prologue(b"SHORT").unwrap()
        .build_responder().unwrap();

    let mut buf = [0u8; 256];
    let mut payload = [0u8; 256];
    let len = initiator.write_message(&[], &mut buf).unwrap();
    responder.read_message(&buf[..len], &mut payload).unwrap();
    let len = responder.write_message(&[], &mut buf).unwrap();
    initiator.read_message(&buf[..len], &mut payload).unwrap();

    let h_r = responder.into_stateless_transport_mode().unwrap();

    // Ciphertext shorter than TAGLEN (16 bytes)
    let short = vec![0u8; 10];
    let mut dec = vec![0u8; 10];
    assert!(h_r.read_message(0, &short, &mut dec).is_err(),
        "ciphertext shorter than tag must be rejected");
}

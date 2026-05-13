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

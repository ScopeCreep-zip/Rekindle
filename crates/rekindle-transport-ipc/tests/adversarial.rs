//! Adversarial tests: every attack vector the transport must reject.
//!
//! These tests prove the transport fails correctly — not silently,
//! not with garbage data, not with a hang. Every attack produces a
//! specific, diagnosable error.

mod common;

use std::sync::Arc;

use rekindle_transport_ipc::bulk::BulkCounters;
use rekindle_transport_ipc::bulk::cipher::BulkCipher;
use rekindle_transport_ipc::bulk::dispatcher::{BulkDispatcher, DecryptedChunk, DispatchError, DEFAULT_REASSEMBLY_CAPACITY};
use rekindle_transport_ipc::bulk::encrypt::build_encrypt_pool;
use rekindle_transport_ipc::bulk::frame::{BulkFrameHeader, FrameKind, HEADER_LEN, TAG_LEN};
use rekindle_transport_ipc::bulk::replay::ReplayFilter;
use rekindle_transport_ipc::bulk::verify::DigestAlgorithm;
use rekindle_transport_ipc::noise::keys::generate_keypair;
use rekindle_transport_ipc::noise::{client_handshake, server_handshake};
use rekindle_transport_ipc::socket::PeerCredentials;

use tokio::sync::mpsc;

// ---- Helper: build a valid encrypted bulk frame ----

fn make_encrypted_frame(
    cipher: &BulkCipher,
    stream_id: u8,
    kind: FrameKind,
    nonce: u64,
    plaintext: &[u8],
) -> Vec<u8> {
    let header = BulkFrameHeader::new(stream_id, kind, nonce, nonce as u32);
    let hdr = header.encode_array();
    let mut frame = Vec::new();
    frame.extend_from_slice(&hdr);
    frame.extend_from_slice(plaintext);
    let ct_start = HEADER_LEN;
    let ct_len = plaintext.len();
    let tag = cipher
        .seal_in_place(nonce, &hdr, &mut frame[ct_start..ct_start + ct_len])
        .unwrap();
    frame.extend_from_slice(&tag);
    frame
}

// ---- 1. Bit-flip in ciphertext: AEAD tag verification rejects ----

#[test]
fn bitflip_in_ciphertext_rejected() {
    common::init_tracing();
    let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
    let pool = build_encrypt_pool(0);
    let (tx, mut rx) = mpsc::channel::<DecryptedChunk>(DEFAULT_REASSEMBLY_CAPACITY);

    let mut dispatcher = BulkDispatcher::new(
        Arc::clone(&cipher), pool, tx, DigestAlgorithm::Blake3, BulkCounters::new(),
    );

    let mut frame = make_encrypted_frame(&cipher, 0, FrameKind::BulkData, 0, b"secret data");

    // Flip one bit in the ciphertext region (after the 14-byte header).
    frame[HEADER_LEN] ^= 0xFF;

    // Dispatch succeeds (frame format is valid). Decrypt fails inside rayon.
    dispatcher.dispatch(frame).unwrap();

    // The decrypted chunk must have decrypt_failed=true and empty plaintext.
    let chunk = rx.blocking_recv().unwrap();
    assert!(chunk.decrypt_failed, "tampered ciphertext must set decrypt_failed");
    assert!(chunk.plaintext.is_empty(), "tampered frame must produce empty plaintext");
}

// ---- 2. Bit-flip in AEAD tag: verification rejects ----

#[test]
fn bitflip_in_tag_rejected() {
    common::init_tracing();
    let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
    let pool = build_encrypt_pool(0);
    let (tx, mut rx) = mpsc::channel::<DecryptedChunk>(DEFAULT_REASSEMBLY_CAPACITY);

    let mut dispatcher = BulkDispatcher::new(
        Arc::clone(&cipher), pool, tx, DigestAlgorithm::Blake3, BulkCounters::new(),
    );

    let mut frame = make_encrypted_frame(&cipher, 0, FrameKind::BulkData, 0, b"secret data");

    // Flip a bit in the last byte (the AEAD tag).
    let last = frame.len() - 1;
    frame[last] ^= 0x01;

    dispatcher.dispatch(frame).unwrap();

    let chunk = rx.blocking_recv().unwrap();
    assert!(chunk.decrypt_failed, "tampered tag must set decrypt_failed");
}

// ---- 3. Nonce replay: replay filter rejects ----

#[test]
fn nonce_replay_rejected_by_dispatcher() {
    common::init_tracing();
    let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
    let pool = build_encrypt_pool(0);
    let (tx, _rx) = mpsc::channel::<DecryptedChunk>(DEFAULT_REASSEMBLY_CAPACITY);

    let mut dispatcher = BulkDispatcher::new(
        Arc::clone(&cipher), pool, tx, DigestAlgorithm::Blake3, BulkCounters::new(),
    );

    let frame = make_encrypted_frame(&cipher, 0, FrameKind::BulkData, 42, b"data");

    // First dispatch: accepted.
    dispatcher.dispatch(frame.clone()).unwrap();

    // Second dispatch with same nonce: rejected.
    let err = dispatcher.dispatch(frame).unwrap_err();
    match err {
        DispatchError::Replay(n) => assert_eq!(n, 42),
        other => panic!("expected Replay(42), got {other:?}"),
    }
}

#[test]
fn nonce_replay_rejected_by_filter_directly() {
    common::init_tracing();
    let mut filter = ReplayFilter::new();

    filter.check_and_accept(0).unwrap();
    filter.check_and_accept(1).unwrap();
    filter.check_and_accept(2).unwrap();

    // Replay of already-seen nonces.
    assert!(filter.check_and_accept(0).is_err());
    assert!(filter.check_and_accept(1).is_err());
    assert!(filter.check_and_accept(2).is_err());

    // New nonce still accepted.
    filter.check_and_accept(3).unwrap();
}

// ---- 4. Wrong key: decrypt fails ----

#[test]
fn wrong_key_rejected() {
    common::init_tracing();
    let cipher_a = BulkCipher::new(&[0x42; 32]);
    let cipher_b = Arc::new(BulkCipher::new(&[0x43; 32])); // different key

    let pool = build_encrypt_pool(0);
    let (tx, mut rx) = mpsc::channel::<DecryptedChunk>(DEFAULT_REASSEMBLY_CAPACITY);

    let mut dispatcher = BulkDispatcher::new(
        Arc::clone(&cipher_b), pool, tx, DigestAlgorithm::Blake3, BulkCounters::new(),
    );

    // Encrypt with key A, dispatch to dispatcher with key B.
    let frame = make_encrypted_frame(&cipher_a, 0, FrameKind::BulkData, 0, b"encrypted with wrong key");
    dispatcher.dispatch(frame).unwrap();

    let chunk = rx.blocking_recv().unwrap();
    assert!(chunk.decrypt_failed, "wrong key must produce decrypt_failed");
    assert!(chunk.plaintext.is_empty());
}

// ---- 5. Truncated frame: too short to contain header + tag ----

#[test]
fn truncated_frame_rejected() {
    common::init_tracing();
    let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
    let pool = build_encrypt_pool(0);
    let (tx, _rx) = mpsc::channel::<DecryptedChunk>(DEFAULT_REASSEMBLY_CAPACITY);

    let mut dispatcher = BulkDispatcher::new(
        Arc::clone(&cipher), pool, tx, DigestAlgorithm::Blake3, BulkCounters::new(),
    );

    // Frame shorter than HEADER_LEN + TAG_LEN = 30 bytes.
    let short_frame = vec![0u8; 20];
    let err = dispatcher.dispatch(short_frame).unwrap_err();
    match err {
        DispatchError::TooShort(20) => {} // correct
        other => panic!("expected TooShort(20), got {other:?}"),
    }
}

// ---- 6. Frame with only header + tag, zero plaintext ----

#[test]
fn empty_plaintext_frame_accepted() {
    common::init_tracing();
    let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
    let pool = build_encrypt_pool(0);
    let (tx, mut rx) = mpsc::channel::<DecryptedChunk>(DEFAULT_REASSEMBLY_CAPACITY);

    let mut dispatcher = BulkDispatcher::new(
        Arc::clone(&cipher), pool, tx, DigestAlgorithm::Blake3, BulkCounters::new(),
    );

    // Encrypt empty plaintext.
    let frame = make_encrypted_frame(&cipher, 0, FrameKind::BulkData, 0, b"");
    dispatcher.dispatch(frame).unwrap();

    let chunk = rx.blocking_recv().unwrap();
    assert!(!chunk.decrypt_failed);
    assert!(chunk.plaintext.is_empty());
}

// ---- 7. Invalid header kind byte ----

#[test]
fn invalid_header_kind_rejected() {
    common::init_tracing();
    let cipher = Arc::new(BulkCipher::new(&[0x42; 32]));
    let pool = build_encrypt_pool(0);
    let (tx, _rx) = mpsc::channel::<DecryptedChunk>(DEFAULT_REASSEMBLY_CAPACITY);

    let mut dispatcher = BulkDispatcher::new(
        Arc::clone(&cipher), pool, tx, DigestAlgorithm::Blake3, BulkCounters::new(),
    );

    // Manually construct a frame with an invalid kind byte.
    let mut frame = vec![0u8; HEADER_LEN + TAG_LEN + 10]; // enough bytes
    frame[0] = 0; // stream_id
    frame[1] = 0xFF; // invalid kind
    // Rest is garbage but frame is long enough.

    let err = dispatcher.dispatch(frame).unwrap_err();
    match err {
        DispatchError::InvalidHeader => {} // correct
        other => panic!("expected InvalidHeader, got {other:?}"),
    }
}

// ---- 8. Wrong nonce on decrypt: AEAD rejects ----

#[test]
fn wrong_nonce_rejected() {
    common::init_tracing();
    let cipher = BulkCipher::new(&[0x42; 32]);
    let mut buf = vec![0xAB; 1024];
    let tag = cipher.seal_in_place(0, b"aad", &mut buf).unwrap();

    let mut combined = Vec::with_capacity(buf.len() + TAG_LEN);
    combined.extend_from_slice(&buf);
    combined.extend_from_slice(&tag);

    // Decrypt with wrong nonce.
    assert!(
        cipher.open_in_place(1, b"aad", &mut combined).is_err(),
        "wrong nonce must fail AEAD verification"
    );
}

// ---- 9. Wrong AAD on decrypt: AEAD rejects ----

#[test]
fn wrong_aad_rejected() {
    common::init_tracing();
    let cipher = BulkCipher::new(&[0x42; 32]);
    let mut buf = vec![0xAB; 1024];
    let tag = cipher.seal_in_place(0, b"correct-aad", &mut buf).unwrap();

    let mut combined = Vec::with_capacity(buf.len() + TAG_LEN);
    combined.extend_from_slice(&buf);
    combined.extend_from_slice(&tag);

    assert!(
        cipher.open_in_place(0, b"wrong-aad", &mut combined).is_err(),
        "wrong AAD must fail AEAD verification"
    );
}

// ---- 10. Noise handshake with wrong server public key ----

#[tokio::test]
async fn wrong_server_pubkey_fails_handshake() {
    common::init_tracing();
    let server_kp = generate_keypair().unwrap();
    let client_kp = generate_keypair().unwrap();
    let wrong_kp = generate_keypair().unwrap();
    // Client uses wrong_kp.public instead of server_kp.public.
    let wrong_pub: [u8; 32] = wrong_kp.public().try_into().unwrap();

    let sc = PeerCredentials { pid: 1, uid: 1000 };
    let cc = PeerCredentials { pid: 2, uid: 1000 };

    let (cs, ss) = tokio::io::duplex(65536);
    let (mut cr, mut cw) = tokio::io::split(cs);
    let (mut sr, mut sw) = tokio::io::split(ss);

    let (client_result, server_result) = tokio::join!(
        client_handshake(&mut cr, &mut cw, &wrong_pub, client_kp.as_inner(), &cc, &sc, std::time::Duration::from_secs(5), rekindle_transport_ipc::frame::codec::MAX_FRAME_SIZE),
        server_handshake(&mut sr, &mut sw, server_kp.as_inner(), &sc, &cc, std::time::Duration::from_secs(5), rekindle_transport_ipc::frame::codec::MAX_FRAME_SIZE),
    );

    // At least one side must fail. The client encrypts to the wrong key,
    // so the server cannot decrypt msg1 → handshake fails.
    assert!(
        client_result.is_err() || server_result.is_err(),
        "wrong server pubkey must fail handshake"
    );
}

// ---- 11. Replay filter: old nonce outside window ----

#[test]
fn nonce_outside_window_rejected() {
    common::init_tracing();
    let mut filter = ReplayFilter::new();

    // Advance to nonce 2000.
    filter.check_and_accept(2000).unwrap();

    // Nonce 0 is > 1024 positions behind. Must be rejected as too old.
    assert!(
        filter.check_and_accept(0).is_err(),
        "nonce outside window must be rejected"
    );
}

// ---- 12. Replay filter: large gap resets window ----

#[test]
fn large_nonce_gap_resets_window() {
    common::init_tracing();
    let mut filter = ReplayFilter::new();

    filter.check_and_accept(0).unwrap();
    filter.check_and_accept(5000).unwrap();

    // Nonce 0 is now outside the window — must be rejected.
    assert!(filter.check_and_accept(0).is_err());

    // Nonce 4999 is inside the new window — must be accepted.
    filter.check_and_accept(4999).unwrap();

    // But not twice.
    assert!(filter.check_and_accept(4999).is_err());
}

// ---- 13. Transport frame tag parsing: invalid app frame ----

#[test]
fn parse_application_frame_rejects_short_payload() {
    common::init_tracing();
    use rekindle_transport_ipc::transport_frame::parse_application_frame;

    // Too short: needs at least 9 bytes (1 tag + 8 seq).
    assert!(parse_application_frame(&[0x80]).is_none());
    assert!(parse_application_frame(&[0x80, 0, 0, 0]).is_none());
    assert!(parse_application_frame(&[]).is_none());

    // Wrong tag.
    assert!(parse_application_frame(&[0x01, 0, 0, 0, 0, 0, 0, 0, 0]).is_none());
}

// ---- 14. BulkNackReason wire roundtrip with chunk_seq: u32 ----

#[test]
fn bulk_nack_reason_roundtrip_decrypt_failed() {
    common::init_tracing();
    use rekindle_transport_ipc::transport_frame::{encode_bulk_nack, BulkNackReason, TransportTag};

    let reason = BulkNackReason::DecryptFailed { chunk_seq: 42 };
    let encoded = encode_bulk_nack(7, &reason);

    // Wire format: [BULK_NACK tag][stream_id][postcard-encoded reason]
    assert_eq!(encoded[0], TransportTag::BULK_NACK);
    assert_eq!(encoded[1], 7); // stream_id

    // Decode the reason back.
    let decoded: BulkNackReason = postcard::from_bytes(&encoded[2..]).unwrap();
    match decoded {
        BulkNackReason::DecryptFailed { chunk_seq } => {
            assert_eq!(chunk_seq, 42, "chunk_seq must roundtrip correctly (u32, not u64)");
        }
        other => panic!("expected DecryptFailed, got {other:?}"),
    }
}

#[test]
fn bulk_nack_reason_roundtrip_all_variants() {
    common::init_tracing();
    use rekindle_transport_ipc::transport_frame::{encode_bulk_nack, BulkNackReason};

    for (reason, label) in [
        (BulkNackReason::DigestMismatch, "DigestMismatch"),
        (BulkNackReason::DecryptFailed { chunk_seq: 0 }, "DecryptFailed(0)"),
        (BulkNackReason::DecryptFailed { chunk_seq: u32::MAX }, "DecryptFailed(MAX)"),
        (BulkNackReason::ReassemblyOverflow, "ReassemblyOverflow"),
        (BulkNackReason::Cancelled, "Cancelled"),
    ] {
        let encoded = encode_bulk_nack(0, &reason);
        let decoded: BulkNackReason = postcard::from_bytes(&encoded[2..])
            .unwrap_or_else(|e| panic!("{label}: decode failed: {e}"));
        // Verify discriminant matches (Debug comparison).
        assert_eq!(
            format!("{decoded:?}"), format!("{reason:?}"),
            "{label}: roundtrip mismatch"
        );
    }
}

#[test]
fn parse_application_frame_accepts_minimal() {
    common::init_tracing();
    use rekindle_transport_ipc::transport_frame::{parse_application_frame, tag_application_frame};

    // Exactly 9 bytes: tag + seq + empty payload.
    let tagged = tag_application_frame(7, b"");
    let (seq, payload) = parse_application_frame(&tagged).unwrap();
    assert_eq!(seq, 7);
    assert!(payload.is_empty());
}

use rekindle_transport_ipc::v3::codec::aead::FrameCipher;
use rekindle_transport_ipc::v3::codec::envelope::{build_envelope, EnvelopeInfo};
use rekindle_transport_ipc::v3::codec::header::{build_header, StreamHeaderInfo};
use rekindle_transport_ipc::v3::crypto::keys::derive_all_keys;
use rekindle_transport_ipc::v3::io::decode::{DecodeError, FrameDecoder};
use rekindle_transport_ipc::v3::wire::constants::{
    WIRE_VERSION, ENVELOPE_LEN, STREAM_HEADER_LEN, DIRECTION_ID_D2L,
};
use rekindle_transport_ipc::v3::wire::frame_class::FrameClass;
use rekindle_transport_ipc::v3::wire::frame_kind::StreamKind;
use rekindle_transport_ipc::v3::wire::lane::Lane;

fn test_keys() -> rekindle_transport_ipc::v3::crypto::keys::DerivedKeys {
    derive_all_keys(&[0xAA; 32])
}

fn make_decoder(keys: &rekindle_transport_ipc::v3::crypto::keys::DerivedKeys) -> FrameDecoder {
    let cipher = FrameCipher::aes256gcm(&keys.stream_d2l, DIRECTION_ID_D2L)
        .expect("key init");
    FrameDecoder::new(keys.envelope_d2l, keys.header_d2l, cipher)
}

fn make_cipher(keys: &rekindle_transport_ipc::v3::crypto::keys::DerivedKeys) -> FrameCipher {
    FrameCipher::aes256gcm(&keys.stream_d2l, DIRECTION_ID_D2L).expect("key init")
}

fn valid_envelope(keys: &rekindle_transport_ipc::v3::crypto::keys::DerivedKeys, seq: u64) -> [u8; ENVELOPE_LEN] {
    build_envelope(
        &EnvelopeInfo {
            wire_version: WIRE_VERSION,
            lane: Lane::Control,
            flags: 0,
            body_len: 100,
            session_seq: seq,
        },
        &keys.envelope_d2l,
    )
}

fn valid_data_envelope(keys: &rekindle_transport_ipc::v3::crypto::keys::DerivedKeys, seq: u64, body_len: u32) -> [u8; ENVELOPE_LEN] {
    build_envelope(
        &EnvelopeInfo {
            wire_version: WIRE_VERSION,
            lane: Lane::Data,
            flags: 0,
            body_len,
            session_seq: seq,
        },
        &keys.envelope_d2l,
    )
}

// ── EMAC must be verified before any field is consulted ────────────

#[test]
fn tampered_lane_rejected_as_emac_not_lane_error() {
    let keys = test_keys();
    let mut envelope = valid_envelope(&keys, 1);
    envelope[1] = 0xFF; // tamper lane
    let mut decoder = make_decoder(&keys);
    let err = decoder.verify_envelope(&envelope).unwrap_err();
    assert!(
        matches!(err, DecodeError::EnvelopeMacFailed),
        "Tampered lane must fail EMAC, not lane-unknown. Got: {err:?}"
    );
}

#[test]
fn tampered_body_len_rejected_as_emac_not_size_error() {
    let keys = test_keys();
    let mut envelope = valid_envelope(&keys, 1);
    envelope[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    let mut decoder = make_decoder(&keys);
    let err = decoder.verify_envelope(&envelope).unwrap_err();
    assert!(
        matches!(err, DecodeError::EnvelopeMacFailed),
        "Tampered body_len must fail EMAC. Got: {err:?}"
    );
}

#[test]
fn tampered_session_seq_rejected_as_emac_not_monotonicity_error() {
    let keys = test_keys();
    let mut envelope = valid_envelope(&keys, 1);
    envelope[8..16].copy_from_slice(&999u64.to_le_bytes());
    let mut decoder = make_decoder(&keys);
    let err = decoder.verify_envelope(&envelope).unwrap_err();
    assert!(
        matches!(err, DecodeError::EnvelopeMacFailed),
        "Tampered session_seq must fail EMAC. Got: {err:?}"
    );
}

// ── After EMAC, body_len bounds are checked before allocation ─────

#[test]
fn valid_emac_but_oversized_body_len_rejected_as_frame_too_large() {
    let keys = test_keys();
    let envelope = build_envelope(
        &EnvelopeInfo {
            wire_version: WIRE_VERSION,
            lane: Lane::Control,
            flags: 0,
            body_len: 1_000_000, // > 64 KiB
            session_seq: 1,
        },
        &keys.envelope_d2l,
    );
    let mut decoder = make_decoder(&keys);
    let err = decoder.verify_envelope(&envelope).unwrap_err();
    assert!(
        matches!(err, DecodeError::FrameTooLarge { .. }),
        "Valid EMAC + oversized body_len must be FrameTooLarge. Got: {err:?}"
    );
}

// ── Envelope accepts out-of-order session_seq (parallel encryption) ──

#[test]
fn out_of_order_session_seq_accepted_by_envelope_decoder() {
    let keys = test_keys();
    let mut decoder = make_decoder(&keys);

    // Simulate parallel encryption: seqs arrive 0, 3, 1, 4, 2
    for &seq in &[0u64, 3, 1, 4, 2] {
        let envelope = valid_envelope(&keys, seq);
        decoder.verify_envelope(&envelope)
            .unwrap_or_else(|e| panic!("seq {seq} must be accepted: {e:?}"));
    }
    // last_seen_seq tracks the most recently processed seq, not the highest.
    // The replay filter tracks the highest internally.
    assert_eq!(decoder.last_seen_seq(), Some(2));
}

#[test]
fn in_order_session_seq_accepted_by_envelope_decoder() {
    let keys = test_keys();
    let mut decoder = make_decoder(&keys);

    for seq in 0..10u64 {
        let envelope = valid_envelope(&keys, seq);
        decoder.verify_envelope(&envelope)
            .unwrap_or_else(|e| panic!("seq {seq} must be accepted: {e:?}"));
    }
    assert_eq!(decoder.last_seen_seq(), Some(9));
}

#[test]
fn reassembler_reorders_out_of_order_chunks() {
    use rekindle_transport_ipc::v3::io::lane_channels::PlaintextBuf;
    use rekindle_transport_ipc::v3::stream::reassembler::Reassembler;

    fn ins(r: &mut Reassembler, idx: u32, data: &[u8]) -> Vec<(u32, PlaintextBuf)> {
        let digest = *blake3::hash(data).as_bytes();
        r.insert_with_digest(idx, PlaintextBuf::Owned(data.to_vec()), digest)
    }

    let mut r = Reassembler::new(256);

    let d2 = ins(&mut r, 2, b"chunk2");
    assert!(d2.is_empty(), "chunk 2 buffered — gap at 0");

    let d0 = ins(&mut r, 0, b"chunk0");
    assert_eq!(d0.len(), 1);
    assert_eq!(d0[0].0, 0);
    assert_eq!(&d0[0].1[..], b"chunk0");

    let d3 = ins(&mut r, 3, b"chunk3");
    assert!(d3.is_empty(), "chunk 3 buffered — gap at 1");

    let d1 = ins(&mut r, 1, b"chunk1");
    assert_eq!(d1.len(), 3);
    assert_eq!(d1[0].0, 1);
    assert_eq!(&d1[0].1[..], b"chunk1");
    assert_eq!(d1[1].0, 2);
    assert_eq!(&d1[1].1[..], b"chunk2");
    assert_eq!(d1[2].0, 3);
    assert_eq!(&d1[2].1[..], b"chunk3");

    assert_eq!(r.next_expected(), 4);
    assert_eq!(r.buffered_count(), 0);
}

#[test]
fn reassembler_content_hash_correct_regardless_of_arrival_order() {
    use rekindle_transport_ipc::v3::io::lane_channels::PlaintextBuf;
    use rekindle_transport_ipc::v3::stream::reassembler::Reassembler;

    fn ins(r: &mut Reassembler, idx: u32, data: &[u8]) -> Vec<(u32, PlaintextBuf)> {
        let digest = *blake3::hash(data).as_bytes();
        r.insert_with_digest(idx, PlaintextBuf::Owned(data.to_vec()), digest)
    }

    let chunks: Vec<Vec<u8>> = vec![
        b"aaaa".to_vec(),
        b"bbbb".to_vec(),
        b"cccc".to_vec(),
        b"dddd".to_vec(),
    ];
    let mut hasher = blake3::Hasher::new();
    for chunk in &chunks { hasher.update(blake3::hash(chunk).as_bytes()); }
    let expected_hash = *hasher.finalize().as_bytes();

    let mut r = Reassembler::new(256);
    ins(&mut r, 3, &chunks[3]);
    ins(&mut r, 1, &chunks[1]);
    ins(&mut r, 0, &chunks[0]);
    ins(&mut r, 2, &chunks[2]);

    assert_eq!(r.next_expected(), 4);
    assert!(r.verify_content_hash(&expected_hash).is_ok(),
        "content hash must match regardless of chunk arrival order");
}

#[test]
fn audit_chain_reorder_buffer_advances_in_session_seq_order() {
    use rekindle_transport_ipc::v3::audit::chain::{AuditChain, LinkInput};

    let key = [0xAA; 32];
    let anchor = [0xBB; 32];

    // Build the reference chain in sequential order
    let mut reference_chain = AuditChain::new(key, anchor);
    let inputs: Vec<LinkInput> = (0..5u64).map(|seq| LinkInput {
        session_seq: seq,
        envelope_hash: [seq as u8; 32],
        header_hash: [(seq + 10) as u8; 32],
        ciphertext_hash: [(seq + 20) as u8; 32],
    }).collect();
    for input in &inputs {
        reference_chain.advance(*input);
    }
    let expected_link = reference_chain.current_link();

    // Simulate reorder buffer: inputs arrive 3, 0, 4, 1, 2
    let arrival_order = [3usize, 0, 4, 1, 2];
    let mut buffer = std::collections::BTreeMap::<u64, LinkInput>::new();
    let mut next_expected: u64 = 0;
    let mut chain = AuditChain::new(key, anchor);

    for &idx in &arrival_order {
        buffer.insert(inputs[idx].session_seq, inputs[idx]);
        while let Some(entry) = buffer.remove(&next_expected) {
            chain.advance(entry);
            next_expected += 1;
        }
    }

    assert_eq!(chain.current_link(), expected_link,
        "reorder buffer must produce identical chain regardless of arrival order");
    assert_eq!(next_expected, 5);
    assert!(buffer.is_empty());
}

// ── HeaderMAC verified before stream_id or frame_kind dispatch ────

#[test]
fn tampered_stream_id_in_header_rejected_as_header_mac() {
    let keys = test_keys();
    let hdr_info = StreamHeaderInfo {
        frame_class: FrameClass::Stream,
        frame_kind: StreamKind::Payload,
        stream_id: 5,
        header_flags: 0,
        chunk_index: 0,
        nonce: 1,
    };
    let mut header = build_header(&hdr_info, &keys.header_d2l);
    header[2] = 99; // tamper stream_id

    let cipher = make_cipher(&keys);
    let envelope = valid_data_envelope(&keys, 1, 100);
    let ciphertext = cipher.seal(1, &envelope, Some(&header), b"payload");

    let mut decoder = make_decoder(&keys);
    decoder.verify_envelope(&envelope).expect("envelope ok");
    let env_info = rekindle_transport_ipc::v3::codec::envelope::parse_envelope(
        &envelope, &keys.envelope_d2l,
    ).expect("parse ok");

    let mut body = Vec::new();
    body.extend_from_slice(&header);
    body.extend_from_slice(&ciphertext);

    let err = decoder.decode_body(&envelope, &env_info, &body).unwrap_err();
    assert!(
        matches!(err, DecodeError::HeaderMacFailed),
        "Tampered stream_id must fail HeaderMAC. Got: {err:?}"
    );
}

#[test]
fn tampered_frame_kind_in_header_rejected_as_header_mac() {
    let keys = test_keys();
    let hdr_info = StreamHeaderInfo {
        frame_class: FrameClass::Stream,
        frame_kind: StreamKind::Payload,
        stream_id: 5,
        header_flags: 0,
        chunk_index: 0,
        nonce: 1,
    };
    let mut header = build_header(&hdr_info, &keys.header_d2l);
    header[1] = StreamKind::Fin as u8; // tamper frame_kind

    let cipher = make_cipher(&keys);
    let envelope = valid_data_envelope(&keys, 2, 100);
    let ciphertext = cipher.seal(1, &envelope, Some(&header), b"payload");

    let mut decoder = make_decoder(&keys);
    // Advance past seq 1 so seq 2 is monotonic
    let env1 = valid_data_envelope(&keys, 1, 100);
    let _ = decoder.verify_envelope(&env1);
    decoder.verify_envelope(&envelope).expect("envelope ok");
    let env_info = rekindle_transport_ipc::v3::codec::envelope::parse_envelope(
        &envelope, &keys.envelope_d2l,
    ).expect("parse ok");

    let mut body = Vec::new();
    body.extend_from_slice(&header);
    body.extend_from_slice(&ciphertext);

    let err = decoder.decode_body(&envelope, &env_info, &body).unwrap_err();
    assert!(
        matches!(err, DecodeError::HeaderMacFailed),
        "Tampered frame_kind must fail HeaderMAC. Got: {err:?}"
    );
}

// ── AEAD verified after both MACs pass ────────────────────────────

#[test]
fn valid_macs_but_tampered_ciphertext_rejected_as_aead() {
    let keys = test_keys();
    let plaintext = b"test payload";
    let hdr_info = StreamHeaderInfo {
        frame_class: FrameClass::Stream,
        frame_kind: StreamKind::Payload,
        stream_id: 0,
        header_flags: 0,
        chunk_index: 0,
        nonce: 1,
    };
    let header = build_header(&hdr_info, &keys.header_d2l);

    let body_len = (STREAM_HEADER_LEN + plaintext.len() + 16) as u32;
    let envelope = valid_data_envelope(&keys, 1, body_len);

    let cipher = make_cipher(&keys);
    let mut ciphertext = cipher.seal(1, &envelope, Some(&header), plaintext);
    if let Some(byte) = ciphertext.first_mut() {
        *byte ^= 0xFF;
    }

    let mut decoder = make_decoder(&keys);
    decoder.verify_envelope(&envelope).expect("envelope ok");
    let env_info = rekindle_transport_ipc::v3::codec::envelope::parse_envelope(
        &envelope, &keys.envelope_d2l,
    ).expect("parse ok");

    let mut body = Vec::new();
    body.extend_from_slice(&header);
    body.extend_from_slice(&ciphertext);

    let err = decoder.decode_body(&envelope, &env_info, &body).unwrap_err();
    assert!(
        matches!(err, DecodeError::AeadVerificationFailed),
        "Valid MACs + tampered ciphertext must fail AEAD. Got: {err:?}"
    );
}

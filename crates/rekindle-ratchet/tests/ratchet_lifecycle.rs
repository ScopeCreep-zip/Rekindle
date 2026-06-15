//! Ratchet lifecycle enforcement tests.
//!
//! Every test asserts ONLY on observable behavior: encrypt → decrypt → plaintext matches.
//! No internal state assertions. No implementation-specific field checks.
//! If the ratchet is wrong, these tests fail with AeadOpen. There is no way to
//! pass them with a broken implementation.

use rekindle_ratchet::crypto::dh;
use rekindle_ratchet::ratchet::{ec, triple};
use rekindle_ratchet::session::{
    Direction, DoubleRatchetState, TrustLevel, TripleRatchetSession,
};
use zeroize::Zeroizing;

// ═══════════════════════════════════════════════════════════════════
// Test infrastructure
// ═══════════════════════════════════════════════════════════════════

struct TestSkipped {
    keys: std::sync::Mutex<Vec<([u8; 32], u32, Zeroizing<[u8; 32]>)>>,
}

impl TestSkipped {
    fn new() -> Self {
        Self { keys: std::sync::Mutex::new(Vec::new()) }
    }
}

impl ec::SkippedKeyCallback for TestSkipped {
    fn store_skipped(
        &self, hk: &[u8; 32], n: u32, mk: &Zeroizing<[u8; 32]>,
    ) -> Result<(), rekindle_ratchet::error::RatchetError> {
        self.keys.lock().unwrap().push((*hk, n, mk.clone()));
        Ok(())
    }

    fn take_skipped(
        &self, hk: &[u8; 32], n: u32,
    ) -> Result<Option<Zeroizing<[u8; 32]>>, rekindle_ratchet::error::RatchetError> {
        let mut keys = self.keys.lock().unwrap();
        if let Some(pos) = keys.iter().position(|(h, c, _)| h == hk && *c == n) {
            let (_, _, mk) = keys.remove(pos);
            Ok(Some(mk))
        } else {
            Ok(None)
        }
    }
}

fn pub_from_seed(seed: &[u8; 32]) -> [u8; 32] {
    let key = dh::reusable_from_seed(seed).unwrap();
    let pk = key.compute_public_key().unwrap();
    let mut buf = [0u8; 32];
    buf.copy_from_slice(pk.as_ref());
    buf
}

fn create_session_pair_with_sk(sk: &Zeroizing<[u8; 32]>) -> (TripleRatchetSession, TripleRatchetSession) {
    let dr_seed = Zeroizing::new([1u8; 32]);
    let dr_pub = pub_from_seed(&dr_seed);
    let spk_seed = Zeroizing::new([2u8; 32]);
    let spk_pub = pub_from_seed(&spk_seed);

    let init_ec = DoubleRatchetState::init_initiator(sk, dr_seed, dr_pub, spk_pub).unwrap();
    let resp_ec = DoubleRatchetState::init_responder(sk.clone(), spk_seed, spk_pub, dr_pub).unwrap();

    let initiator = TripleRatchetSession::new([0u8; 32], Direction::Initiator, init_ec, TrustLevel::Untrusted);
    let responder = TripleRatchetSession::new([0u8; 32], Direction::Responder, resp_ec, TrustLevel::Untrusted);
    (initiator, responder)
}

fn create_session_pair() -> (TripleRatchetSession, TripleRatchetSession) {
    create_session_pair_with_sk(&Zeroizing::new([42u8; 32]))
}

// ═══════════════════════════════════════════════════════════════════
// CONFORMANCE — encrypt → decrypt → plaintext matches
// ═══════════════════════════════════════════════════════════════════

#[test]
fn initiator_sends_responder_decrypts() {
    let (mut init, mut resp) = create_session_pair();
    let skipped = TestSkipped::new();

    let msg = triple::encrypt(&mut init, b"hello from initiator").unwrap();
    let pt = triple::decrypt(&mut resp, &msg.encrypted_header, &msg.ciphertext, &skipped).unwrap();
    assert_eq!(pt, b"hello from initiator");
}

#[test]
fn responder_sends_initiator_decrypts() {
    let (mut init, mut resp) = create_session_pair();
    let skipped = TestSkipped::new();

    let msg = triple::encrypt(&mut resp, b"hello from responder").unwrap();
    let pt = triple::decrypt(&mut init, &msg.encrypted_header, &msg.ciphertext, &skipped).unwrap();
    assert_eq!(pt, b"hello from responder");
}

#[test]
fn responder_sends_first_then_full_volley() {
    let (mut init, mut resp) = create_session_pair();
    let skipped = TestSkipped::new();

    let msg1 = triple::encrypt(&mut resp, b"responder goes first").unwrap();
    let pt1 = triple::decrypt(&mut init, &msg1.encrypted_header, &msg1.ciphertext, &skipped).unwrap();
    assert_eq!(pt1, b"responder goes first");

    let msg2 = triple::encrypt(&mut init, b"initiator replies").unwrap();
    let pt2 = triple::decrypt(&mut resp, &msg2.encrypted_header, &msg2.ciphertext, &skipped).unwrap();
    assert_eq!(pt2, b"initiator replies");

    let msg3 = triple::encrypt(&mut resp, b"responder again").unwrap();
    let pt3 = triple::decrypt(&mut init, &msg3.encrypted_header, &msg3.ciphertext, &skipped).unwrap();
    assert_eq!(pt3, b"responder again");

    for i in 0..5 {
        let pa = format!("volley init {i}");
        let ma = triple::encrypt(&mut init, pa.as_bytes()).unwrap();
        let pta = triple::decrypt(&mut resp, &ma.encrypted_header, &ma.ciphertext, &skipped).unwrap();
        assert_eq!(pta, pa.as_bytes());

        let pb = format!("volley resp {i}");
        let mb = triple::encrypt(&mut resp, pb.as_bytes()).unwrap();
        let ptb = triple::decrypt(&mut init, &mb.encrypted_header, &mb.ciphertext, &skipped).unwrap();
        assert_eq!(ptb, pb.as_bytes());
    }
}

#[test]
fn bidirectional_volley_10() {
    let (mut init, mut resp) = create_session_pair();
    let skipped = TestSkipped::new();

    for i in 0..5 {
        let pa = format!("init msg {i}");
        let msg = triple::encrypt(&mut init, pa.as_bytes()).unwrap();
        let pt = triple::decrypt(&mut resp, &msg.encrypted_header, &msg.ciphertext, &skipped).unwrap();
        assert_eq!(pt, pa.as_bytes());

        let pb = format!("resp msg {i}");
        let msg = triple::encrypt(&mut resp, pb.as_bytes()).unwrap();
        let pt = triple::decrypt(&mut init, &msg.encrypted_header, &msg.ciphertext, &skipped).unwrap();
        assert_eq!(pt, pb.as_bytes());
    }
}

#[test]
fn burst_5_messages_same_chain() {
    let (mut init, mut resp) = create_session_pair();
    let skipped = TestSkipped::new();

    for i in 0..5 {
        let payload = format!("burst {i}");
        let msg = triple::encrypt(&mut init, payload.as_bytes()).unwrap();
        let pt = triple::decrypt(&mut resp, &msg.encrypted_header, &msg.ciphertext, &skipped).unwrap();
        assert_eq!(pt, payload.as_bytes());
    }
}

#[test]
fn pqxdh_init_encrypt_he_decrypt_he_then_dm() {
    let sk = Zeroizing::new([42u8; 32]);
    let dr_seed = Zeroizing::new([1u8; 32]);
    let dr_pub = pub_from_seed(&dr_seed);
    let spk_seed = Zeroizing::new([2u8; 32]);
    let spk_pub = pub_from_seed(&spk_seed);

    let mut init_ec = DoubleRatchetState::init_initiator(&sk, dr_seed, dr_pub, spk_pub).unwrap();

    let mut proof = Vec::with_capacity(74);
    proof.extend_from_slice(b"PQXDH-INIT");
    proof.extend_from_slice(&[0xAA; 32]);
    proof.extend_from_slice(&[0xBB; 32]);

    let msg = ec::encrypt_he(&mut init_ec, &proof).unwrap();

    let mut resp_ec = DoubleRatchetState::init_responder(sk, spk_seed, spk_pub, dr_pub).unwrap();
    let skipped = TestSkipped::new();
    let plaintext = ec::decrypt_he(&mut resp_ec, &msg.encrypted_header, &msg.ciphertext, &skipped).unwrap();

    assert_eq!(&plaintext[..10], b"PQXDH-INIT");
    assert_eq!(&plaintext[10..42], &[0xAA; 32]);
    assert_eq!(&plaintext[42..74], &[0xBB; 32]);

    // After PQXDH-INIT, real DMs work via triple
    let mut init_session = TripleRatchetSession::new([0u8; 32], Direction::Initiator, init_ec, TrustLevel::Untrusted);
    let mut resp_session = TripleRatchetSession::new([0u8; 32], Direction::Responder, resp_ec, TrustLevel::Untrusted);

    let real_msg = triple::encrypt(&mut init_session, b"first real DM").unwrap();
    let pt = triple::decrypt(&mut resp_session, &real_msg.encrypted_header, &real_msg.ciphertext, &skipped).unwrap();
    assert_eq!(pt, b"first real DM");

    // Responder replies
    let reply = triple::encrypt(&mut resp_session, b"responder reply after pqxdh").unwrap();
    let pt2 = triple::decrypt(&mut init_session, &reply.encrypted_header, &reply.ciphertext, &skipped).unwrap();
    assert_eq!(pt2, b"responder reply after pqxdh");
}

// ═══════════════════════════════════════════════════════════════════
// ADVERSARIAL — malicious input rejected
// ═══════════════════════════════════════════════════════════════════

#[test]
fn tampered_ciphertext_rejected() {
    let (mut init, mut resp) = create_session_pair();
    let skipped = TestSkipped::new();

    let msg = triple::encrypt(&mut init, b"secret").unwrap();
    let mut bad_ct = msg.ciphertext.clone();
    bad_ct[0] ^= 0xFF;

    let result = triple::decrypt(&mut resp, &msg.encrypted_header, &bad_ct, &skipped);
    assert!(result.is_err());
}

#[test]
fn tampered_header_rejected() {
    let (mut init, mut resp) = create_session_pair();
    let skipped = TestSkipped::new();

    let msg = triple::encrypt(&mut init, b"secret").unwrap();
    let mut bad_header = msg.encrypted_header.clone();
    bad_header[0] ^= 0xFF;

    let result = triple::decrypt(&mut resp, &bad_header, &msg.ciphertext, &skipped);
    assert!(result.is_err());
}

#[test]
fn replay_rejected() {
    let (mut init, mut resp) = create_session_pair();
    let skipped = TestSkipped::new();

    let msg = triple::encrypt(&mut init, b"one-time").unwrap();
    triple::decrypt(&mut resp, &msg.encrypted_header, &msg.ciphertext, &skipped).unwrap();
    let result = triple::decrypt(&mut resp, &msg.encrypted_header, &msg.ciphertext, &skipped);
    assert!(result.is_err());
}

#[test]
fn wrong_session_rejected() {
    let (mut init_a, _) = create_session_pair_with_sk(&Zeroizing::new([42u8; 32]));
    let (_, mut resp_b) = create_session_pair_with_sk(&Zeroizing::new([99u8; 32]));
    let skipped = TestSkipped::new();

    let msg = triple::encrypt(&mut init_a, b"wrong target").unwrap();
    let result = triple::decrypt(&mut resp_b, &msg.encrypted_header, &msg.ciphertext, &skipped);
    assert!(result.is_err());
}

// ═══════════════════════════════════════════════════════════════════
// ANTAGONISTIC — hostile conditions
// ═══════════════════════════════════════════════════════════════════

#[test]
fn concurrent_send_both_sides() {
    let (mut init, mut resp) = create_session_pair();
    let skipped = TestSkipped::new();

    // Setup: one message each direction to establish chains
    let setup = triple::encrypt(&mut init, b"setup").unwrap();
    triple::decrypt(&mut resp, &setup.encrypted_header, &setup.ciphertext, &skipped).unwrap();

    // Both send before receiving
    let msg_init = triple::encrypt(&mut init, b"from init").unwrap();
    let msg_resp = triple::encrypt(&mut resp, b"from resp").unwrap();

    // Both receive
    let pt_resp = triple::decrypt(&mut resp, &msg_init.encrypted_header, &msg_init.ciphertext, &skipped).unwrap();
    let pt_init = triple::decrypt(&mut init, &msg_resp.encrypted_header, &msg_resp.ciphertext, &skipped).unwrap();
    assert_eq!(pt_resp, b"from init");
    assert_eq!(pt_init, b"from resp");

    // Continue after crossing
    let msg3 = triple::encrypt(&mut init, b"after cross").unwrap();
    let pt3 = triple::decrypt(&mut resp, &msg3.encrypted_header, &msg3.ciphertext, &skipped).unwrap();
    assert_eq!(pt3, b"after cross");
}

#[test]
fn responder_sends_first_then_crossing() {
    let (mut init, mut resp) = create_session_pair();
    let skipped = TestSkipped::new();

    // Responder sends first
    let msg_resp = triple::encrypt(&mut resp, b"resp first").unwrap();
    // Initiator sends without receiving resp's message
    let msg_init = triple::encrypt(&mut init, b"init concurrent").unwrap();

    // Both receive the other's message
    let pt_init = triple::decrypt(&mut resp, &msg_init.encrypted_header, &msg_init.ciphertext, &skipped).unwrap();
    let pt_resp = triple::decrypt(&mut init, &msg_resp.encrypted_header, &msg_resp.ciphertext, &skipped).unwrap();
    assert_eq!(pt_init, b"init concurrent");
    assert_eq!(pt_resp, b"resp first");

    // Continue after crossing resolves
    let msg3 = triple::encrypt(&mut init, b"after crossing").unwrap();
    let pt3 = triple::decrypt(&mut resp, &msg3.encrypted_header, &msg3.ciphertext, &skipped).unwrap();
    assert_eq!(pt3, b"after crossing");

    let msg4 = triple::encrypt(&mut resp, b"resp after crossing").unwrap();
    let pt4 = triple::decrypt(&mut init, &msg4.encrypted_header, &msg4.ciphertext, &skipped).unwrap();
    assert_eq!(pt4, b"resp after crossing");
}

// ═══════════════════════════════════════════════════════════════════
// DURABILITY — crash/restart cycles
// ═══════════════════════════════════════════════════════════════════

#[test]
fn serialization_roundtrip() {
    let (mut init, mut resp) = create_session_pair();
    let skipped = TestSkipped::new();

    let msg = triple::encrypt(&mut init, b"before serialize").unwrap();
    triple::decrypt(&mut resp, &msg.encrypted_header, &msg.ciphertext, &skipped).unwrap();

    let cbor = cbor4ii::serde::to_vec(Vec::new(), &resp).unwrap();
    let mut resp2: TripleRatchetSession = cbor4ii::serde::from_slice(&cbor).unwrap();

    let msg2 = triple::encrypt(&mut resp2, b"after deserialize").unwrap();
    let pt = triple::decrypt(&mut init, &msg2.encrypted_header, &msg2.ciphertext, &skipped).unwrap();
    assert_eq!(pt, b"after deserialize");
}

#[test]
fn crash_after_encrypt_restore_and_retry() {
    let (mut init, mut resp) = create_session_pair();
    let skipped = TestSkipped::new();

    let cbor_before = cbor4ii::serde::to_vec(Vec::new(), &init).unwrap();
    let _lost = triple::encrypt(&mut init, b"lost message").unwrap();

    let mut init_restored: TripleRatchetSession = cbor4ii::serde::from_slice(&cbor_before).unwrap();
    let msg = triple::encrypt(&mut init_restored, b"retry message").unwrap();
    let pt = triple::decrypt(&mut resp, &msg.encrypted_header, &msg.ciphertext, &skipped).unwrap();
    assert_eq!(pt, b"retry message");
}

#[test]
fn hundred_messages_with_persist_each() {
    let (mut init, mut resp) = create_session_pair();
    let skipped = TestSkipped::new();

    for i in 0u32..100 {
        let payload = format!("msg {i}");
        let msg = triple::encrypt(&mut init, payload.as_bytes()).unwrap();

        let init_cbor = cbor4ii::serde::to_vec(Vec::new(), &init).unwrap();
        init = cbor4ii::serde::from_slice(&init_cbor).unwrap();

        let pt = triple::decrypt(&mut resp, &msg.encrypted_header, &msg.ciphertext, &skipped).unwrap();
        assert_eq!(pt, payload.as_bytes());

        let resp_cbor = cbor4ii::serde::to_vec(Vec::new(), &resp).unwrap();
        resp = cbor4ii::serde::from_slice(&resp_cbor).unwrap();
    }
}

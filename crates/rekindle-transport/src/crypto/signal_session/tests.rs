//! Session-manager tests, including the CROSS-TRACK interop suite
//! (daemon-track manager talking to the desktop-track manager over
//! the shared ratchet core and storage traits).

use super::*;
use crate::crypto::signal_store::{MemoryIdentityStore, MemoryPreKeyStore, MemorySessionStore};
use parking_lot::Mutex;
use rekindle_crypto::error::CryptoError;
use std::collections::HashMap;
use std::sync::Arc;

struct SharedSessionStore(Arc<Mutex<HashMap<String, Vec<u8>>>>);

// The storage traits are the shared `rekindle-crypto` definitions,
// so this fixture speaks their error type (`CryptoError`), not the
// transport's.
impl SessionStore for SharedSessionStore {
    fn load_session(&self, address: &str) -> std::result::Result<Option<Vec<u8>>, CryptoError> {
        Ok(self.0.lock().get(address).cloned())
    }
    fn store_session(&self, address: &str, data: &[u8]) -> std::result::Result<(), CryptoError> {
        self.0.lock().insert(address.to_string(), data.to_vec());
        Ok(())
    }
    fn has_session(&self, address: &str) -> std::result::Result<bool, CryptoError> {
        Ok(self.0.lock().contains_key(address))
    }
    fn delete_session(&self, address: &str) -> std::result::Result<(), CryptoError> {
        self.0.lock().remove(address);
        Ok(())
    }
    fn list_sessions(&self) -> std::result::Result<Vec<String>, CryptoError> {
        Ok(self.0.lock().keys().cloned().collect())
    }
}

/// Ed25519 identity keypair — bytes match production layout
/// (`MemoryIdentityStore` holds the Ed25519 32-byte secret + Ed25519
/// 32-byte public). PQXDH derives X25519 from Ed25519 internally via
/// `to_scalar_bytes()`, matching `Identity::to_x25519_secret`.
fn make_identity() -> (Vec<u8>, Vec<u8>) {
    let signing = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
    let verifying = signing.verifying_key();
    (signing.to_bytes().to_vec(), verifying.to_bytes().to_vec())
}

fn establish_pair() -> (SignalSessionManager, SignalSessionManager, String, String) {
    let (alice_priv, alice_pub) = make_identity();
    let (bob_priv, bob_pub) = make_identity();

    let alice = SignalSessionManager::new(
        Box::new(MemoryIdentityStore::new(alice_priv, alice_pub.clone(), 1)),
        Box::new(MemoryPreKeyStore::new()),
        Box::new(SharedSessionStore(Arc::new(Mutex::new(HashMap::new())))),
    );
    let bob = SignalSessionManager::new(
        Box::new(MemoryIdentityStore::new(bob_priv, bob_pub.clone(), 2)),
        Box::new(MemoryPreKeyStore::new()),
        Box::new(SharedSessionStore(Arc::new(Mutex::new(HashMap::new())))),
    );

    let alice_addr = hex::encode(&alice_pub);
    let bob_addr = hex::encode(&bob_pub);

    let bob_bundle = bob.generate_prekey_bundle(1, Some(100), Some(100)).unwrap();
    let init = alice.establish_session(&bob_addr, &bob_bundle).unwrap();

    bob.respond_to_session(
        &alice_addr,
        &alice_pub,
        &init.ephemeral_public_key,
        init.signed_prekey_id,
        init.one_time_prekey_id,
        &init.ml_kem_ciphertext,
        init.used_ot_pqpk_id,
    )
    .unwrap();

    (alice, bob, alice_addr, bob_addr)
}

#[test]
fn x3dh_encrypt_decrypt_roundtrip() {
    let (alice, bob, alice_addr, bob_addr) = establish_pair();

    let ct = alice.encrypt(&bob_addr, b"hello bob").unwrap();
    let pt = bob.decrypt(&alice_addr, &ct).unwrap();
    assert_eq!(pt, b"hello bob");

    let reply_ct = bob.encrypt(&alice_addr, b"hi alice").unwrap();
    let reply_pt = alice.decrypt(&bob_addr, &reply_ct).unwrap();
    assert_eq!(reply_pt, b"hi alice");
}

#[test]
fn multiple_messages_different_ciphertexts() {
    let (alice, bob, alice_addr, bob_addr) = establish_pair();

    let c1 = alice.encrypt(&bob_addr, b"msg1").unwrap();
    let c2 = alice.encrypt(&bob_addr, b"msg2").unwrap();
    assert_ne!(c1, c2);

    assert_eq!(bob.decrypt(&alice_addr, &c1).unwrap(), b"msg1");
    assert_eq!(bob.decrypt(&alice_addr, &c2).unwrap(), b"msg2");
}

#[test]
fn prekey_bundle_generation() {
    let (priv_key, pub_key) = make_identity();
    let mgr = SignalSessionManager::new(
        Box::new(MemoryIdentityStore::new(priv_key, pub_key, 1)),
        Box::new(MemoryPreKeyStore::new()),
        Box::new(MemorySessionStore::new()),
    );

    let bundle = mgr.generate_prekey_bundle(1, Some(100), Some(100)).unwrap();
    assert_eq!(bundle.signed_prekey.len(), 32);
    assert_eq!(bundle.signed_prekey_signature.len(), 64);
    assert!(bundle.one_time_prekey.is_some());
    assert_eq!(bundle.registration_id, 1);
    // PQXDH additions: ML-KEM-768 last-resort bundle is always present.
    assert_eq!(bundle.pqpk_lr.len(), 1184);
    assert_eq!(bundle.pqpk_lr_signature.len(), 64);
    assert!(bundle.pqpk_ot.is_some());
}

#[test]
fn encrypt_without_session_fails() {
    let (priv_key, pub_key) = make_identity();
    let mgr = SignalSessionManager::new(
        Box::new(MemoryIdentityStore::new(priv_key, pub_key, 1)),
        Box::new(MemoryPreKeyStore::new()),
        Box::new(MemorySessionStore::new()),
    );
    assert!(mgr.encrypt("nobody", b"hello").is_err());
}

/// CROSS-TRACK INTEROP — the reason the shared ratchet core exists.
///
/// Alice runs the daemon track's manager (this file); Bob runs the
/// desktop track's (`rekindle_crypto::signal::session`). Before the
/// ratchets were converged this could not work: the desktop wrote
/// `[counter || nonce || ct]` with a symmetric-only chain while the
/// daemon wrote `[ratchet_public || counter || nonce || ct]` with a
/// per-message DH step — a DM between the two tracks could not
/// decrypt. This test is the regression guard that keeps the tracks
/// on ONE wire format and ONE key schedule.
#[tokio::test]
async fn cross_track_session_interop() {
    use rekindle_crypto::signal::memory_stores as desktop_stores;
    use rekindle_crypto::signal::session::SignalSessionManager as DesktopManager;

    let (alice_priv, alice_pub) = make_identity();
    let (bob_priv, bob_pub) = make_identity();

    // Alice: daemon-track manager (transport stores).
    let alice = SignalSessionManager::new(
        Box::new(MemoryIdentityStore::new(alice_priv, alice_pub.clone(), 1)),
        Box::new(MemoryPreKeyStore::new()),
        Box::new(MemorySessionStore::new()),
    );
    // Bob: desktop-track manager (rekindle-crypto stores).
    let bob = DesktopManager::new(
        Box::new(desktop_stores::MemoryIdentityStore::new(
            bob_priv,
            bob_pub.clone(),
            2,
        )),
        Box::new(desktop_stores::MemoryPreKeyStore::new()),
        Box::new(desktop_stores::MemorySessionStore::new()),
    );

    let alice_addr = hex::encode(&alice_pub);
    let bob_addr = hex::encode(&bob_pub);

    // Bob (desktop) publishes a bundle; Alice (daemon) initiates.
    let bob_bundle = bob.generate_prekey_bundle(1, Some(100), Some(100)).unwrap();
    let init = alice.establish_session(&bob_addr, &bob_bundle).unwrap();
    bob.respond_to_session(
        &alice_addr,
        &alice_pub,
        &init.ephemeral_public_key,
        init.signed_prekey_id,
        init.one_time_prekey_id,
        &init.ml_kem_ciphertext,
        init.used_ot_pqpk_id,
    )
    .unwrap();

    // Daemon → desktop.
    let wire = alice
        .encrypt(&bob_addr, b"hello from the daemon track")
        .unwrap();
    assert_eq!(
        bob.decrypt(&alice_addr, &wire).await.unwrap(),
        b"hello from the daemon track"
    );

    // Desktop → daemon.
    let wire = bob
        .encrypt(&alice_addr, b"hello from the desktop track")
        .await
        .unwrap();
    assert_eq!(
        alice.decrypt(&bob_addr, &wire).unwrap(),
        b"hello from the desktop track"
    );

    // A short ping-pong to prove the DH ratchet stays in step
    // across implementations, not just on the first exchange.
    for i in 0..4u8 {
        let m = vec![i; 32];
        if i % 2 == 0 {
            let w = alice.encrypt(&bob_addr, &m).unwrap();
            assert_eq!(bob.decrypt(&alice_addr, &w).await.unwrap(), m);
        } else {
            let w = bob.encrypt(&alice_addr, &m).await.unwrap();
            assert_eq!(alice.decrypt(&bob_addr, &w).unwrap(), m);
        }
    }
}

/// Responder-replies-first also works cross-track — this ordering
/// was broken in BOTH pre-convergence forks (each stored the
/// initiator's ephemeral PUBLIC key where the ratchet secret
/// belongs, so the initiator could never decrypt a first inbound).
#[tokio::test]
async fn cross_track_responder_sends_first() {
    use rekindle_crypto::signal::memory_stores as desktop_stores;
    use rekindle_crypto::signal::session::SignalSessionManager as DesktopManager;

    let (alice_priv, alice_pub) = make_identity();
    let (bob_priv, bob_pub) = make_identity();

    let alice = SignalSessionManager::new(
        Box::new(MemoryIdentityStore::new(alice_priv, alice_pub.clone(), 1)),
        Box::new(MemoryPreKeyStore::new()),
        Box::new(MemorySessionStore::new()),
    );
    let bob = DesktopManager::new(
        Box::new(desktop_stores::MemoryIdentityStore::new(
            bob_priv,
            bob_pub.clone(),
            2,
        )),
        Box::new(desktop_stores::MemoryPreKeyStore::new()),
        Box::new(desktop_stores::MemorySessionStore::new()),
    );

    let alice_addr = hex::encode(&alice_pub);
    let bob_addr = hex::encode(&bob_pub);

    let bob_bundle = bob.generate_prekey_bundle(1, None, None).unwrap();
    let init = alice.establish_session(&bob_addr, &bob_bundle).unwrap();
    bob.respond_to_session(
        &alice_addr,
        &alice_pub,
        &init.ephemeral_public_key,
        init.signed_prekey_id,
        init.one_time_prekey_id,
        &init.ml_kem_ciphertext,
        init.used_ot_pqpk_id,
    )
    .unwrap();

    // Bob (the responder) speaks FIRST.
    let wire = bob.encrypt(&alice_addr, b"responder first").await.unwrap();
    assert_eq!(alice.decrypt(&bob_addr, &wire).unwrap(), b"responder first");
}

/// Concurrent sends to one peer must not reuse a message key.
///
/// The Double Ratchet spec advances the sending chain as
/// `state.CKs, mk = KDF_CK(state.CKs)` then `state.Ns += 1`, and requires
/// that "every message sent or received is encrypted with a unique
/// message key". `encrypt` loads the serialised session, steps it, and
/// stores it back — so without per-peer serialisation two threads both
/// step from the same `CKs` and derive the same key. This drove the
/// `peer_locks` map; before it, the two ciphertexts below shared a
/// counter.
#[test]
fn concurrent_encrypt_to_one_peer_never_reuses_a_message_key() {
    use std::sync::Arc;

    let (alice, _bob, _alice_addr, bob_addr) = establish_pair();
    let alice = Arc::new(alice);

    let mut handles = Vec::new();
    for i in 0..8u8 {
        let mgr = Arc::clone(&alice);
        let peer = bob_addr.clone();
        handles.push(std::thread::spawn(move || {
            mgr.encrypt(&peer, &[i; 16]).expect("encrypt")
        }));
    }
    let cts: Vec<Vec<u8>> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    // Counter is bytes 32..40 of the wire format
    // `[ratchet_public(32) || counter(8 LE) || nonce(12) || ct]`.
    let counters: std::collections::BTreeSet<u64> = cts
        .iter()
        .map(|c| u64::from_le_bytes(c[32..40].try_into().unwrap()))
        .collect();
    assert_eq!(
        counters.len(),
        cts.len(),
        "every concurrent encrypt must consume a distinct chain counter"
    );
}

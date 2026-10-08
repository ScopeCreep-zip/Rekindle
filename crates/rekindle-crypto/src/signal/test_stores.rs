//! Signal-session test support.
//!
//! The in-memory `IdentityKeyStore` / `PreKeyStore` implementations used
//! here are the ones in `memory_stores`, not copies of them. This module
//! previously re-declared both verbatim (~127 lines) alongside the real
//! shared-store fixture and tests, so a fix to one copy silently missed
//! the other.

use std::collections::HashMap;

use parking_lot::Mutex;

use super::memory_stores::{MemoryIdentityStore, MemoryPreKeyStore};
use crate::signal::store::SessionStore;
use crate::CryptoError;

/// A SessionStore backed by a shared HashMap, allowing test code to inspect stored data.
struct SharedSessionStore(std::sync::Arc<Mutex<HashMap<String, Vec<u8>>>>);

impl SessionStore for SharedSessionStore {
    fn load_session(&self, address: &str) -> Result<Option<Vec<u8>>, CryptoError> {
        Ok(self.0.lock().get(address).cloned())
    }

    fn store_session(&self, address: &str, session_data: &[u8]) -> Result<(), CryptoError> {
        self.0
            .lock()
            .insert(address.to_string(), session_data.to_vec());
        Ok(())
    }

    fn has_session(&self, address: &str) -> Result<bool, CryptoError> {
        Ok(self.0.lock().contains_key(address))
    }

    fn delete_session(&self, address: &str) -> Result<(), CryptoError> {
        self.0.lock().remove(address);
        Ok(())
    }

    fn list_sessions(&self) -> Result<Vec<String>, CryptoError> {
        Ok(self.0.lock().keys().cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::signal::SignalSessionManager;
    use crate::Identity;

    /// Create a basic SignalSessionManager for a given identity (no shared store).
    ///
    /// PQXDH (Phase 3b) needs Ed25519 identity bytes in the store so the
    /// session manager can re-derive X25519 via `to_scalar_bytes` internally.
    /// Storing X25519 bytes here would double-derive and produce mismatched keys.
    fn make_manager(identity: &Identity) -> SignalSessionManager {
        SignalSessionManager::new(
            Box::new(MemoryIdentityStore::new(
                identity.secret_key_bytes().to_vec(),
                identity.public_key_bytes().to_vec(),
                1,
            )),
            Box::new(MemoryPreKeyStore::new()),
            Box::new(SharedSessionStore(Arc::new(Mutex::new(HashMap::new())))),
        )
    }

    /// PQXDH initiator + responder round-trip.
    ///
    /// Alice initiates, Bob responds. Returns (alice_mgr, bob_mgr, alice_addr, bob_addr).
    fn establish_session_pair() -> (SignalSessionManager, SignalSessionManager, String, String) {
        let alice_id = Identity::generate();
        let bob_id = Identity::generate();

        let alice_mgr = make_manager(&alice_id);
        let bob_mgr = make_manager(&bob_id);

        let alice_addr = hex::encode(alice_id.public_key_bytes());
        let bob_addr = hex::encode(bob_id.public_key_bytes());

        // Step 1: Bob hands Alice a PQXDH prekey bundle (classical + ML-KEM,
        // with one-time keys).
        let bob_bundle = bob_mgr.handout_bundle().unwrap();

        // Step 2: Alice establishes session as initiator — derives root key
        // from PQXDH (DH1..DH4 + ML-KEM encapsulation).
        let init = alice_mgr.establish_session(&bob_addr, &bob_bundle).unwrap();

        // Step 3: Bob responds with Alice's ephemeral + ML-KEM ciphertext.
        bob_mgr
            .respond_to_session(
                &alice_addr,
                &alice_id.public_key_bytes(),
                &init.ephemeral_public_key,
                init.signed_prekey_id,
                init.one_time_prekey_id,
                &init.ml_kem_ciphertext,
                init.used_ot_pqpk_id,
            )
            .unwrap();

        (alice_mgr, bob_mgr, alice_addr, bob_addr)
    }

    #[tokio::test]
    async fn x3dh_handshake_and_encrypt_decrypt() {
        let (alice_mgr, bob_mgr, alice_addr, bob_addr) = establish_session_pair();

        // Alice encrypts for Bob
        let plaintext = b"Hello Bob, this is a secret message!";
        let ciphertext = alice_mgr.encrypt(&bob_addr, plaintext).await.unwrap();

        // Bob decrypts
        let decrypted = bob_mgr.decrypt(&alice_addr, &ciphertext).await.unwrap();
        assert_eq!(decrypted, plaintext);

        // Bob replies
        let reply = b"Hi Alice, received your message!";
        let reply_ct = bob_mgr.encrypt(&alice_addr, reply).await.unwrap();
        let reply_pt = alice_mgr.decrypt(&bob_addr, &reply_ct).await.unwrap();
        assert_eq!(reply_pt, reply);
    }

    #[tokio::test]
    async fn multiple_messages_advance_chain() {
        let (alice_mgr, bob_mgr, alice_addr, bob_addr) = establish_session_pair();

        let msg1 = alice_mgr.encrypt(&bob_addr, b"message 1").await.unwrap();
        let msg2 = alice_mgr.encrypt(&bob_addr, b"message 2").await.unwrap();
        let msg3 = alice_mgr.encrypt(&bob_addr, b"message 3").await.unwrap();

        // Each ciphertext must differ (different chain keys per message)
        assert_ne!(msg1, msg2);
        assert_ne!(msg2, msg3);

        // Decrypt in order
        assert_eq!(
            bob_mgr.decrypt(&alice_addr, &msg1).await.unwrap(),
            b"message 1"
        );
        assert_eq!(
            bob_mgr.decrypt(&alice_addr, &msg2).await.unwrap(),
            b"message 2"
        );
        assert_eq!(
            bob_mgr.decrypt(&alice_addr, &msg3).await.unwrap(),
            b"message 3"
        );
    }

    #[tokio::test]
    async fn wrong_key_decryption_fails() {
        let (alice_mgr, _bob_mgr, alice_addr, bob_addr) = establish_session_pair();

        let eve_id = Identity::generate();
        let eve_mgr = make_manager(&eve_id);

        // Alice encrypts for Bob
        let ciphertext = alice_mgr
            .encrypt(&bob_addr, b"secret for Bob")
            .await
            .unwrap();

        // Eve has no session with Alice — decryption fails
        let result = eve_mgr.decrypt(&alice_addr, &ciphertext).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn tampered_ciphertext_fails() {
        let (alice_mgr, bob_mgr, alice_addr, bob_addr) = establish_session_pair();

        let ciphertext = alice_mgr
            .encrypt(&bob_addr, b"don't tamper with me")
            .await
            .unwrap();

        // Flip a byte in the ciphertext portion (after 8-byte counter + 12-byte nonce)
        let mut tampered = ciphertext.clone();
        if tampered.len() > 20 {
            tampered[20] ^= 0xFF;
        }

        let result = bob_mgr.decrypt(&alice_addr, &tampered).await;
        assert!(result.is_err());
    }

    #[test]
    fn has_session_reports_correctly() {
        let alice_id = Identity::generate();
        let bob_id = Identity::generate();
        let alice_mgr = make_manager(&alice_id);
        let bob_mgr = make_manager(&bob_id);

        let bob_addr = hex::encode(bob_id.public_key_bytes());
        assert!(!alice_mgr.has_session(&bob_addr).unwrap());

        let bob_bundle = bob_mgr.current_bundle().unwrap();
        alice_mgr.establish_session(&bob_addr, &bob_bundle).unwrap();

        assert!(alice_mgr.has_session(&bob_addr).unwrap());
    }

    /// Run the initiator half against `bundle`, then the responder half.
    fn handshake(
        initiator: &SignalSessionManager,
        initiator_id: &Identity,
        responder: &SignalSessionManager,
        responder_id: &Identity,
        bundle: &crate::signal::PreKeyBundle,
    ) -> Result<(), CryptoError> {
        let init =
            initiator.establish_session(&hex::encode(responder_id.public_key_bytes()), bundle)?;
        responder.respond_to_session(
            &hex::encode(initiator_id.public_key_bytes()),
            &initiator_id.public_key_bytes(),
            &init.ephemeral_public_key,
            init.signed_prekey_id,
            init.one_time_prekey_id,
            &init.ml_kem_ciphertext,
            init.used_ot_pqpk_id,
        )
    }

    /// Messages flow both ways over an established pair.
    async fn assert_round_trip(
        a: &SignalSessionManager,
        a_id: &Identity,
        b: &SignalSessionManager,
        b_id: &Identity,
    ) {
        let (a_addr, b_addr) = (
            hex::encode(a_id.public_key_bytes()),
            hex::encode(b_id.public_key_bytes()),
        );
        let ct = a.encrypt(&b_addr, b"ping").await.unwrap();
        assert_eq!(b.decrypt(&a_addr, &ct).await.unwrap(), b"ping");
        let ct = b.encrypt(&a_addr, b"pong").await.unwrap();
        assert_eq!(a.decrypt(&b_addr, &ct).await.unwrap(), b"pong");
    }

    #[test]
    fn bundle_shapes() {
        let identity = Identity::generate();
        let mgr = make_manager(&identity);

        let current = mgr.current_bundle().unwrap();
        assert_eq!(current.identity_key.len(), 32);
        assert_eq!(current.signed_prekey.len(), 32);
        assert_eq!(current.signed_prekey_signature.len(), 64);
        assert_eq!(current.registration_id, 1);
        assert!(current.one_time_prekey.is_none() && current.one_time_prekey_id.is_none());
        assert!(current.pqpk_ot.is_none() && current.pqpk_ot_id.is_none());

        let handout = mgr.handout_bundle().unwrap();
        assert_eq!(handout.signed_prekey, current.signed_prekey);
        assert_eq!(handout.pqpk_lr, current.pqpk_lr);
        assert_eq!(handout.one_time_prekey.as_ref().map(Vec::len), Some(32));
        assert!(handout.one_time_prekey_id.is_some_and(|id| id != 0));
        assert!(handout.pqpk_ot_id.is_some_and(|id| id != 0));
        assert!(handout.pqpk_ot.is_some() && handout.pqpk_ot_signature.is_some());

        let next = mgr.handout_bundle().unwrap();
        assert_ne!(next.one_time_prekey, handout.one_time_prekey);
        assert_ne!(next.one_time_prekey_id, handout.one_time_prekey_id);
        assert_ne!(next.pqpk_ot_id, handout.pqpk_ot_id);
    }

    /// Every handed-out bundle completes exactly once, whatever order the
    /// handshakes arrive in, and a replay of a consumed one fails.
    #[tokio::test]
    async fn handouts_complete_independently() {
        use rand::seq::SliceRandom;

        let bob_id = Identity::generate();
        let bob = make_manager(&bob_id);
        let peers: Vec<(Identity, SignalSessionManager)> = (0..50)
            .map(|_| {
                let id = Identity::generate();
                let mgr = make_manager(&id);
                (id, mgr)
            })
            .collect();
        let bundles: Vec<_> = peers
            .iter()
            .map(|_| bob.handout_bundle().unwrap())
            .collect();

        let mut order: Vec<usize> = (0..peers.len()).collect();
        order.shuffle(&mut rand::rngs::OsRng);
        let (first, rest) = order.split_at(peers.len() / 2);
        for &i in first.iter().chain(rest) {
            let (id, mgr) = &peers[i];
            handshake(mgr, id, &bob, &bob_id, &bundles[i]).unwrap();
            assert_round_trip(mgr, id, &bob, &bob_id).await;
        }

        let (id, mgr) = &peers[first[0]];
        assert!(
            handshake(mgr, id, &bob, &bob_id, &bundles[first[0]]).is_err(),
            "a consumed one-time key must not be usable twice"
        );
    }

    #[test]
    fn current_bundle_is_byte_stable() {
        let bob_id = Identity::generate();
        let bob = make_manager(&bob_id);
        let before = postcard::to_stdvec(&bob.current_bundle().unwrap()).unwrap();
        assert_eq!(
            before,
            postcard::to_stdvec(&bob.current_bundle().unwrap()).unwrap()
        );

        let alice_id = Identity::generate();
        let alice = make_manager(&alice_id);
        let handout = bob.handout_bundle().unwrap();
        handshake(&alice, &alice_id, &bob, &bob_id, &handout).unwrap();

        assert_eq!(
            before,
            postcard::to_stdvec(&bob.current_bundle().unwrap()).unwrap(),
            "handouts and consumption must not touch the long-lived keys"
        );
    }

    /// Beyond the cap the oldest unclaimed one-time keys are deleted; the
    /// newest stay usable.
    #[test]
    fn eviction_keeps_newest_one_time_keys() {
        use crate::signal::MAX_UNCLAIMED_ONE_TIME;

        let bob_id = Identity::generate();
        let bob = make_manager(&bob_id);
        let extra = 3;
        let bundles: Vec<_> = (0..MAX_UNCLAIMED_ONE_TIME + extra)
            .map(|_| bob.handout_bundle().unwrap())
            .collect();

        let alice_id = Identity::generate();
        let alice = make_manager(&alice_id);
        for evicted in &bundles[..extra] {
            assert!(handshake(&alice, &alice_id, &bob, &bob_id, evicted).is_err());
        }
        for kept in [&bundles[extra], bundles.last().unwrap()] {
            handshake(&alice, &alice_id, &bob, &bob_id, kept).unwrap();
        }
    }

    /// Crossing friend requests: exactly one side initiates, the other
    /// answers, and the session works both ways.
    #[tokio::test]
    async fn crossing_requests_pick_one_initiator() {
        let a_id = Identity::generate();
        let b_id = Identity::generate();
        let a = make_manager(&a_id);
        let b = make_manager(&b_id);

        let a_initiates = a
            .initiates_crossing_handshake(&b_id.public_key_bytes())
            .unwrap();
        let b_initiates = b
            .initiates_crossing_handshake(&a_id.public_key_bytes())
            .unwrap();
        assert_ne!(a_initiates, b_initiates, "exactly one side initiates");

        // Each side's request carried a handout bundle.
        let a_bundle = a.handout_bundle().unwrap();
        let b_bundle = b.handout_bundle().unwrap();
        if a_initiates {
            handshake(&a, &a_id, &b, &b_id, &b_bundle).unwrap();
        } else {
            handshake(&b, &b_id, &a, &a_id, &a_bundle).unwrap();
        }
        assert_round_trip(&a, &a_id, &b, &b_id).await;

        assert!(a
            .initiates_crossing_handshake(&a_id.public_key_bytes())
            .is_err());
    }

    #[tokio::test]
    async fn empty_message_encrypt_decrypt() {
        let (alice_mgr, bob_mgr, alice_addr, bob_addr) = establish_session_pair();

        let ct = alice_mgr.encrypt(&bob_addr, b"").await.unwrap();
        let pt = bob_mgr.decrypt(&alice_addr, &ct).await.unwrap();
        assert!(pt.is_empty());
    }

    #[tokio::test]
    async fn decrypt_too_short_message_fails() {
        let (_, bob_mgr, alice_addr, _) = establish_session_pair();

        let result = bob_mgr.decrypt(&alice_addr, &[0u8; 19]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn encrypt_without_session_fails() {
        let alice_id = Identity::generate();
        let alice_mgr = make_manager(&alice_id);

        let result = alice_mgr.encrypt("nonexistent_peer", b"hello").await;
        assert!(result.is_err());
    }
}

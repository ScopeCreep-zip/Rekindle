use std::collections::HashMap;

use parking_lot::Mutex;

use crate::signal::store::{IdentityKeyStore, PqKeyKind, PreKeyStore, SessionStore};
use crate::CryptoError;

/// In-memory identity store for tests.
pub struct MemoryIdentityStore {
    identity_private: Vec<u8>,
    identity_public: Vec<u8>,
    registration_id: u32,
    trusted: Mutex<HashMap<String, Vec<u8>>>,
}

impl MemoryIdentityStore {
    pub fn new(identity_private: Vec<u8>, identity_public: Vec<u8>, registration_id: u32) -> Self {
        Self {
            identity_private,
            identity_public,
            registration_id,
            trusted: Mutex::new(HashMap::new()),
        }
    }
}

impl IdentityKeyStore for MemoryIdentityStore {
    fn get_identity_key_pair(&self) -> Result<(Vec<u8>, Vec<u8>), CryptoError> {
        Ok((self.identity_private.clone(), self.identity_public.clone()))
    }

    fn get_local_registration_id(&self) -> Result<u32, CryptoError> {
        Ok(self.registration_id)
    }

    fn is_trusted_identity(&self, address: &str, identity_key: &[u8]) -> Result<bool, CryptoError> {
        let trusted = self.trusted.lock();
        match trusted.get(address) {
            Some(stored) => Ok(stored == identity_key),
            None => Ok(true), // TOFU: trust on first use
        }
    }

    fn save_identity(&self, address: &str, identity_key: &[u8]) -> Result<(), CryptoError> {
        self.trusted
            .lock()
            .insert(address.to_string(), identity_key.to_vec());
        Ok(())
    }
}

/// In-memory prekey store for tests.
pub struct MemoryPreKeyStore {
    prekeys: Mutex<HashMap<u32, Vec<u8>>>,
    signed_prekeys: Mutex<HashMap<u32, Vec<u8>>>,
    pq_secrets: Mutex<HashMap<(u32, PqKeyKind), Vec<u8>>>,
}

impl MemoryPreKeyStore {
    pub fn new() -> Self {
        Self {
            prekeys: Mutex::new(HashMap::new()),
            signed_prekeys: Mutex::new(HashMap::new()),
            pq_secrets: Mutex::new(HashMap::new()),
        }
    }
}

impl PreKeyStore for MemoryPreKeyStore {
    fn load_prekey(&self, prekey_id: u32) -> Result<Option<Vec<u8>>, CryptoError> {
        Ok(self.prekeys.lock().get(&prekey_id).cloned())
    }

    fn store_prekey(&self, prekey_id: u32, key_data: &[u8]) -> Result<(), CryptoError> {
        self.prekeys.lock().insert(prekey_id, key_data.to_vec());
        Ok(())
    }

    fn remove_prekey(&self, prekey_id: u32) -> Result<(), CryptoError> {
        self.prekeys.lock().remove(&prekey_id);
        Ok(())
    }

    fn load_signed_prekey(&self, signed_prekey_id: u32) -> Result<Option<Vec<u8>>, CryptoError> {
        Ok(self.signed_prekeys.lock().get(&signed_prekey_id).cloned())
    }

    fn store_signed_prekey(
        &self,
        signed_prekey_id: u32,
        key_data: &[u8],
    ) -> Result<(), CryptoError> {
        self.signed_prekeys
            .lock()
            .insert(signed_prekey_id, key_data.to_vec());
        Ok(())
    }

    fn load_pq_secret(
        &self,
        prekey_id: u32,
        kind: PqKeyKind,
    ) -> Result<Option<Vec<u8>>, CryptoError> {
        Ok(self.pq_secrets.lock().get(&(prekey_id, kind)).cloned())
    }

    fn store_pq_secret(
        &self,
        prekey_id: u32,
        kind: PqKeyKind,
        key_data: &[u8],
    ) -> Result<(), CryptoError> {
        self.pq_secrets
            .lock()
            .insert((prekey_id, kind), key_data.to_vec());
        Ok(())
    }

    fn remove_pq_secret(&self, prekey_id: u32, kind: PqKeyKind) -> Result<(), CryptoError> {
        if kind == PqKeyKind::OneTime {
            self.pq_secrets.lock().remove(&(prekey_id, kind));
        }
        Ok(())
    }
}

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

        // Step 1: Bob publishes a PQXDH prekey bundle (classical + ML-KEM).
        let bob_bundle = bob_mgr
            .generate_prekey_bundle(1, Some(100), Some(100))
            .unwrap();

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

        let bob_bundle = bob_mgr.generate_prekey_bundle(1, None, None).unwrap();
        alice_mgr.establish_session(&bob_addr, &bob_bundle).unwrap();

        assert!(alice_mgr.has_session(&bob_addr).unwrap());
    }

    #[test]
    fn prekey_bundle_generation() {
        let identity = Identity::generate();
        let mgr = make_manager(&identity);

        let bundle = mgr.generate_prekey_bundle(1, Some(100), Some(100)).unwrap();
        assert_eq!(bundle.identity_key.len(), 32);
        assert_eq!(bundle.signed_prekey.len(), 32);
        assert_eq!(bundle.signed_prekey_signature.len(), 64);
        assert!(bundle.one_time_prekey.is_some());
        assert_eq!(bundle.one_time_prekey.as_ref().unwrap().len(), 32);
        assert_eq!(bundle.registration_id, 1);

        let bundle2 = mgr.generate_prekey_bundle(2, None, None).unwrap();
        assert!(bundle2.one_time_prekey.is_none());
        assert_ne!(bundle.signed_prekey, bundle2.signed_prekey);
    }

    #[test]
    fn load_existing_prekey_bundle_returns_none_when_empty() {
        // P1.2 — fresh Stronghold-equivalent (Memory* store starts empty)
        // → load_existing_prekey_bundle returns None so caller mints fresh.
        let identity = Identity::generate();
        let mgr = make_manager(&identity);
        let result = mgr
            .load_existing_prekey_bundle(1, Some(1), Some(1))
            .unwrap();
        assert!(result.is_none(), "empty store must return None");
    }

    #[test]
    fn load_existing_prekey_bundle_reuses_after_generate() {
        // P1.2 — after generate_prekey_bundle persists prekey #1 +
        // signed_prekey #1, a subsequent load_existing_prekey_bundle
        // must return the SAME public keys (no fresh generation).
        let identity = Identity::generate();
        let mgr = make_manager(&identity);

        let original = mgr.generate_prekey_bundle(1, Some(1), Some(1)).unwrap();
        let loaded = mgr
            .load_existing_prekey_bundle(1, Some(1), Some(1))
            .unwrap()
            .expect("bundle must be loadable after generate");

        assert_eq!(original.identity_key, loaded.identity_key);
        assert_eq!(original.signed_prekey, loaded.signed_prekey);
        assert_eq!(
            original.one_time_prekey, loaded.one_time_prekey,
            "one-time prekey must be reused, not regenerated"
        );
        assert_eq!(original.registration_id, loaded.registration_id);
        // The signature is deterministic over (identity_private,
        // signed_prekey_public) so re-signing yields the same bytes.
        assert_eq!(
            original.signed_prekey_signature, loaded.signed_prekey_signature,
            "signature must be stable across reuse"
        );
    }

    #[test]
    fn load_existing_prekey_bundle_returns_none_when_otpk_missing() {
        // P1.2 — if the requested one-time prekey ID isn't in the
        // store but the signed prekey is, return None so the caller
        // re-generates rather than building a partial bundle.
        let identity = Identity::generate();
        let mgr = make_manager(&identity);

        // Generate signed_prekey only (no OTPK).
        let _ = mgr.generate_prekey_bundle(1, None, None).unwrap();
        // Asking for OTPK #1 — not present.
        let result = mgr
            .load_existing_prekey_bundle(1, Some(1), Some(1))
            .unwrap();
        assert!(
            result.is_none(),
            "missing one-time prekey must force regeneration"
        );
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

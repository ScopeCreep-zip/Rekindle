use std::collections::HashMap;
use std::sync::Mutex;

use crate::CryptoError;
use crate::signal::store::{IdentityKeyStore, PreKeyStore, SessionStore};

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
        let trusted = self.trusted.lock().unwrap();
        match trusted.get(address) {
            Some(stored) => Ok(stored == identity_key),
            None => Ok(true), // TOFU: trust on first use
        }
    }

    fn save_identity(&self, address: &str, identity_key: &[u8]) -> Result<(), CryptoError> {
        self.trusted.lock().unwrap().insert(address.to_string(), identity_key.to_vec());
        Ok(())
    }
}

/// In-memory prekey store for tests.
pub struct MemoryPreKeyStore {
    prekeys: Mutex<HashMap<u32, Vec<u8>>>,
    signed_prekeys: Mutex<HashMap<u32, Vec<u8>>>,
}

impl MemoryPreKeyStore {
    pub fn new() -> Self {
        Self {
            prekeys: Mutex::new(HashMap::new()),
            signed_prekeys: Mutex::new(HashMap::new()),
        }
    }
}

impl PreKeyStore for MemoryPreKeyStore {
    fn load_prekey(&self, prekey_id: u32) -> Result<Option<Vec<u8>>, CryptoError> {
        Ok(self.prekeys.lock().unwrap().get(&prekey_id).cloned())
    }

    fn store_prekey(&self, prekey_id: u32, key_data: &[u8]) -> Result<(), CryptoError> {
        self.prekeys.lock().unwrap().insert(prekey_id, key_data.to_vec());
        Ok(())
    }

    fn remove_prekey(&self, prekey_id: u32) -> Result<(), CryptoError> {
        self.prekeys.lock().unwrap().remove(&prekey_id);
        Ok(())
    }

    fn load_signed_prekey(&self, signed_prekey_id: u32) -> Result<Option<Vec<u8>>, CryptoError> {
        Ok(self.signed_prekeys.lock().unwrap().get(&signed_prekey_id).cloned())
    }

    fn store_signed_prekey(&self, signed_prekey_id: u32, key_data: &[u8]) -> Result<(), CryptoError> {
        self.signed_prekeys.lock().unwrap().insert(signed_prekey_id, key_data.to_vec());
        Ok(())
    }
}

/// A SessionStore backed by a shared HashMap, allowing test code to inspect stored data.
struct SharedSessionStore(std::sync::Arc<Mutex<HashMap<String, Vec<u8>>>>);

impl SessionStore for SharedSessionStore {
    fn load_session(&self, address: &str) -> Result<Option<Vec<u8>>, CryptoError> {
        Ok(self.0.lock().unwrap().get(address).cloned())
    }

    fn store_session(&self, address: &str, session_data: &[u8]) -> Result<(), CryptoError> {
        self.0.lock().unwrap().insert(address.to_string(), session_data.to_vec());
        Ok(())
    }

    fn has_session(&self, address: &str) -> Result<bool, CryptoError> {
        Ok(self.0.lock().unwrap().contains_key(address))
    }

    fn delete_session(&self, address: &str) -> Result<(), CryptoError> {
        self.0.lock().unwrap().remove(address);
        Ok(())
    }

    fn list_sessions(&self) -> Result<Vec<String>, CryptoError> {
        Ok(self.0.lock().unwrap().keys().cloned().collect())
    }
}

/// Extract the ephemeral public key from a serialized ratchet state.
///
/// Ratchet layout: root_key(32) + sending_chain(32) + receiving_chain(32)
///   + our_ratchet_secret_len(4) + our_ratchet_secret(N) + ...
/// After `establish_session`, `our_ratchet_secret` stores the ephemeral *public* key.
fn extract_ephemeral_from_session(session_data: &[u8]) -> Vec<u8> {
    let pos = 96; // skip root_key + sending + receiving chain keys
    let len = u32::from_le_bytes(session_data[pos..pos + 4].try_into().unwrap()) as usize;
    session_data[pos + 4..pos + 4 + len].to_vec()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::Identity;
    use crate::signal::SignalSessionManager;

    /// Create a basic SignalSessionManager for a given identity (no shared store).
    fn make_manager(identity: &Identity) -> SignalSessionManager {
        let x25519_secret = identity.to_x25519_secret();
        let x25519_public = identity.to_x25519_public();

        SignalSessionManager::new(
            Box::new(MemoryIdentityStore::new(
                x25519_secret.to_bytes().to_vec(),
                x25519_public.as_bytes().to_vec(),
                1,
            )),
            Box::new(MemoryPreKeyStore::new()),
            Box::new(SharedSessionStore(Arc::new(Mutex::new(HashMap::new())))),
        )
    }

    /// Create a proper X3DH session pair using initiator + responder flow.
    ///
    /// Alice initiates, Bob responds. Returns (alice_mgr, bob_mgr, alice_addr, bob_addr).
    fn establish_session_pair() -> (SignalSessionManager, SignalSessionManager, String, String) {
        let alice_id = Identity::generate();
        let bob_id = Identity::generate();

        let alice_x25519_pub = alice_id.to_x25519_public();

        // Use shared session stores so we can inspect Alice's session data
        let alice_sessions = Arc::new(Mutex::new(HashMap::<String, Vec<u8>>::new()));

        let alice_mgr = SignalSessionManager::new(
            Box::new(MemoryIdentityStore::new(
                alice_id.to_x25519_secret().to_bytes().to_vec(),
                alice_x25519_pub.as_bytes().to_vec(),
                1,
            )),
            Box::new(MemoryPreKeyStore::new()),
            Box::new(SharedSessionStore(Arc::clone(&alice_sessions))),
        );

        let bob_mgr = SignalSessionManager::new(
            Box::new(MemoryIdentityStore::new(
                bob_id.to_x25519_secret().to_bytes().to_vec(),
                bob_id.to_x25519_public().as_bytes().to_vec(),
                1,
            )),
            Box::new(MemoryPreKeyStore::new()),
            Box::new(SharedSessionStore(Arc::new(Mutex::new(HashMap::new())))),
        );

        let alice_addr = hex::encode(alice_id.public_key_bytes());
        let bob_addr = hex::encode(bob_id.public_key_bytes());

        // Step 1: Bob publishes a prekey bundle
        let bob_bundle = bob_mgr.generate_prekey_bundle(1, Some(100)).unwrap();

        // Step 2: Alice establishes session as initiator
        alice_mgr.establish_session(&bob_addr, &bob_bundle).unwrap();

        // Step 3: Extract Alice's ephemeral key from her stored session
        let alice_session_data = alice_sessions.lock().unwrap().get(&bob_addr).unwrap().clone();
        let alice_ephemeral = extract_ephemeral_from_session(&alice_session_data);

        // Step 4: Bob responds to session as responder
        bob_mgr
            .respond_to_session(
                &alice_addr,
                alice_x25519_pub.as_bytes(),
                &alice_ephemeral,
                1,
                Some(100),
            )
            .unwrap();

        (alice_mgr, bob_mgr, alice_addr, bob_addr)
    }

    #[test]
    fn x3dh_handshake_and_encrypt_decrypt() {
        let (alice_mgr, bob_mgr, alice_addr, bob_addr) = establish_session_pair();

        // Alice encrypts for Bob
        let plaintext = b"Hello Bob, this is a secret message!";
        let ciphertext = alice_mgr.encrypt(&bob_addr, plaintext).unwrap();

        // Bob decrypts
        let decrypted = bob_mgr.decrypt(&alice_addr, &ciphertext).unwrap();
        assert_eq!(decrypted, plaintext);

        // Bob replies
        let reply = b"Hi Alice, received your message!";
        let reply_ct = bob_mgr.encrypt(&alice_addr, reply).unwrap();
        let reply_pt = alice_mgr.decrypt(&bob_addr, &reply_ct).unwrap();
        assert_eq!(reply_pt, reply);
    }

    #[test]
    fn multiple_messages_advance_chain() {
        let (alice_mgr, bob_mgr, alice_addr, bob_addr) = establish_session_pair();

        let msg1 = alice_mgr.encrypt(&bob_addr, b"message 1").unwrap();
        let msg2 = alice_mgr.encrypt(&bob_addr, b"message 2").unwrap();
        let msg3 = alice_mgr.encrypt(&bob_addr, b"message 3").unwrap();

        // Each ciphertext must differ (different chain keys per message)
        assert_ne!(msg1, msg2);
        assert_ne!(msg2, msg3);

        // Decrypt in order
        assert_eq!(bob_mgr.decrypt(&alice_addr, &msg1).unwrap(), b"message 1");
        assert_eq!(bob_mgr.decrypt(&alice_addr, &msg2).unwrap(), b"message 2");
        assert_eq!(bob_mgr.decrypt(&alice_addr, &msg3).unwrap(), b"message 3");
    }

    #[test]
    fn wrong_key_decryption_fails() {
        let (alice_mgr, _bob_mgr, alice_addr, bob_addr) = establish_session_pair();

        let eve_id = Identity::generate();
        let eve_mgr = make_manager(&eve_id);

        // Alice encrypts for Bob
        let ciphertext = alice_mgr.encrypt(&bob_addr, b"secret for Bob").unwrap();

        // Eve has no session with Alice — decryption fails
        let result = eve_mgr.decrypt(&alice_addr, &ciphertext);
        assert!(result.is_err());
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let (alice_mgr, bob_mgr, alice_addr, bob_addr) = establish_session_pair();

        let ciphertext = alice_mgr.encrypt(&bob_addr, b"don't tamper with me").unwrap();

        // Flip a byte in the ciphertext portion (after 8-byte counter + 12-byte nonce)
        let mut tampered = ciphertext.clone();
        if tampered.len() > 20 {
            tampered[20] ^= 0xFF;
        }

        let result = bob_mgr.decrypt(&alice_addr, &tampered);
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

        let bob_bundle = bob_mgr.generate_prekey_bundle(1, None).unwrap();
        alice_mgr.establish_session(&bob_addr, &bob_bundle).unwrap();

        assert!(alice_mgr.has_session(&bob_addr).unwrap());
    }

    #[test]
    fn prekey_bundle_generation() {
        let identity = Identity::generate();
        let mgr = make_manager(&identity);

        let bundle = mgr.generate_prekey_bundle(1, Some(100)).unwrap();
        assert_eq!(bundle.identity_key.len(), 32);
        assert_eq!(bundle.signed_prekey.len(), 32);
        assert_eq!(bundle.signed_prekey_signature.len(), 64);
        assert!(bundle.one_time_prekey.is_some());
        assert_eq!(bundle.one_time_prekey.as_ref().unwrap().len(), 32);
        assert_eq!(bundle.registration_id, 1);

        let bundle2 = mgr.generate_prekey_bundle(2, None).unwrap();
        assert!(bundle2.one_time_prekey.is_none());
        assert_ne!(bundle.signed_prekey, bundle2.signed_prekey);
    }

    #[test]
    fn empty_message_encrypt_decrypt() {
        let (alice_mgr, bob_mgr, alice_addr, bob_addr) = establish_session_pair();

        let ct = alice_mgr.encrypt(&bob_addr, b"").unwrap();
        let pt = bob_mgr.decrypt(&alice_addr, &ct).unwrap();
        assert!(pt.is_empty());
    }

    #[test]
    fn decrypt_too_short_message_fails() {
        let (_, bob_mgr, alice_addr, _) = establish_session_pair();

        let result = bob_mgr.decrypt(&alice_addr, &[0u8; 19]);
        assert!(result.is_err());
    }

    #[test]
    fn encrypt_without_session_fails() {
        let alice_id = Identity::generate();
        let alice_mgr = make_manager(&alice_id);

        let result = alice_mgr.encrypt("nonexistent_peer", b"hello");
        assert!(result.is_err());
    }
}

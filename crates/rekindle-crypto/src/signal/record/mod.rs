//! `SessionRecord`: every session with one peer device, as a pure value
//! (plan E2.2). Each operation takes `&self` and returns the new record,
//! so the caller stores it in the same transaction as whatever the
//! message changed (plan E2.3), and a failure leaves the stored record as
//! it was.
//!
//! The record is stored under `(peer identity, peer device)`. It holds one
//! active session and up to [`ARCHIVED_STATES_MAX_LENGTH`] inactive ones,
//! newest first (libsignal `SessionRecord`, `consts.rs`). Sesame's rules
//! apply: a new session becomes active and the old one is archived; a
//! message that only an inactive session decrypts promotes it (libsignal
//! `promote_old_session`); and a PreKeyInit whose base key is already a
//! session's is a no-op, reported as [`CryptoError::DuplicateMessage`]
//! (libsignal `promote_matching_session`, r10 B4).
//!
//! Identity == address: the record is for one identity key, and a bundle
//! or init under any other key is refused with
//! [`CryptoError::IdentityAddressMismatch`].

mod codec;
#[cfg(test)]
mod proptests;
#[cfg(test)]
mod tests;

use rekindle_utils::random::RandomSource;

use crate::bytes::to_32;
use crate::error::CryptoError;
use crate::signal::dr::{RatchetMessage, SessionState};
use crate::signal::pqxdh::agreement::{self, PqPrekeyId, ResponderKeys};
use crate::signal::pqxdh::PqxdhError;
use crate::signal::prekeys::PreKeyBundle;
use crate::Identity;

/// Inactive sessions kept per record (libsignal
/// `ARCHIVED_STATES_MAX_LENGTH`).
pub const ARCHIVED_STATES_MAX_LENGTH: usize = 40;

/// The initiator's first message (PQXDH rev. 3 §3.3): its identity, its
/// ephemeral key, the KEM ciphertext, the prekey ids it used, and the
/// first ratchet message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreKeyInit {
    /// `IK_A`, the initiator's Ed25519 identity (its address).
    pub identity_key: [u8; 32],
    /// `EK_A`'s public half: the session's base key.
    pub base_key: [u8; 32],
    /// The responder's signed prekey the init used.
    pub signed_prekey_id: u32,
    /// The responder's one-time curve prekey, when DH4 ran.
    pub one_time_prekey_id: Option<u32>,
    /// The responder's ML-KEM prekey `CT` is for.
    pub pq_prekey: PqPrekeyId,
    /// `CT`.
    pub kem_ciphertext: Vec<u8>,
    /// The first message, encrypted under the new session.
    pub message: RatchetMessage,
}

/// The sessions with one peer device.
#[derive(Clone)]
pub struct SessionRecord {
    peer: [u8; 32],
    active: Option<SessionState>,
    /// Newest first.
    inactive: Vec<SessionState>,
}

impl SessionRecord {
    /// A record for `peer_identity` with no session yet.
    #[must_use]
    pub fn new(peer_identity: [u8; 32]) -> Self {
        Self {
            peer: peer_identity,
            active: None,
            inactive: Vec::new(),
        }
    }

    /// The identity key (address) this record is for.
    pub fn peer_identity(&self) -> &[u8; 32] {
        &self.peer
    }

    /// Whether a session is active, so [`Self::encrypt`] can send.
    pub fn has_active(&self) -> bool {
        self.active.is_some()
    }

    /// Whether a session in this record has `base_key`: a PreKeyInit
    /// with it is a duplicate, known before its prekeys are looked up
    /// (a consumed one-time key no longer exists to look up).
    pub fn has_base_key(&self, base_key: &[u8; 32]) -> bool {
        self.states().any(|(_, s)| s.base_key() == base_key)
    }

    /// The base key of the active session while it is a PreKeyInit of ours
    /// the peer has not answered (nothing decrypted under it yet): the one
    /// a `PreKeyRejected` may discard (plan E2.4, B6).
    pub fn pending_init(&self) -> Option<&[u8; 32]> {
        self.active
            .as_ref()
            .filter(|s| !s.has_received())
            .map(SessionState::base_key)
    }

    /// This record without the pending init `base_key` (the peer refused
    /// its prekeys); `None` when it is not the pending init. The session
    /// archived before it becomes active again.
    #[must_use]
    pub fn without_pending(&self, base_key: &[u8; 32]) -> Option<Self> {
        if self.pending_init() != Some(base_key) {
            return None;
        }
        let mut record = self.clone();
        record.active = (!record.inactive.is_empty()).then(|| record.inactive.remove(0));
        Some(record)
    }

    /// Start a session from the peer's bundle and encrypt the first
    /// message under it (the initiator side). The new session is active;
    /// the old one, if any, is archived.
    pub fn initiate(
        &self,
        our: &Identity,
        bundle: &PreKeyBundle,
        first_plaintext: &[u8],
        ad_extra: &[u8],
        rng: &dyn RandomSource,
    ) -> Result<(Self, PreKeyInit), CryptoError> {
        self.check_address(our, &bundle.identity_key)?;
        let agreed = agreement::initiate(our, bundle, rng).map_err(|e| pqxdh_error(&e))?;
        let mut state = SessionState::initiator(
            &agreed.keys,
            agreed.ad,
            agreed.base_key,
            to_32(&bundle.signed_prekey, "signed prekey")?,
            rng,
        )?;
        let message = state.encrypt(first_plaintext, ad_extra)?;
        let init = PreKeyInit {
            identity_key: our.public_key_bytes(),
            base_key: agreed.base_key,
            signed_prekey_id: bundle.signed_prekey_id,
            one_time_prekey_id: agreed.one_time_prekey_id,
            pq_prekey: agreed.pq_prekey,
            kem_ciphertext: agreed.kem_ciphertext,
            message,
        };
        Ok((self.with_active(state), init))
    }

    /// Accept a peer's PreKeyInit (the responder side): build the session
    /// from our secrets and decrypt the first message. Only a first
    /// message that decrypts yields a record, so the caller consumes the
    /// one-time prekeys the init names only on `Ok` (X3DH §3.4).
    pub fn insert_from_init(
        &self,
        init: &PreKeyInit,
        keys: &ResponderKeys<'_>,
        ad_extra: &[u8],
        now_ms: i64,
        rng: &dyn RandomSource,
    ) -> Result<(Self, Vec<u8>), CryptoError> {
        self.check_address(keys.identity, &init.identity_key)?;
        if self.has_base_key(&init.base_key) {
            return Err(CryptoError::DuplicateMessage);
        }
        if init.one_time_prekey_id.is_some() != keys.one_time_prekey.is_some() {
            return Err(CryptoError::invalid_key(
                "the one-time prekey does not match the init".into(),
            ));
        }
        let initiator = ed25519_dalek::VerifyingKey::from_bytes(&init.identity_key)
            .map_err(|e| CryptoError::invalid_key(format!("initiator identity: {e}")))?;
        let (session_keys, ad) =
            agreement::respond(keys, &initiator, &init.base_key, &init.kem_ciphertext)
                .map_err(|e| pqxdh_error(&e))?;
        let mut state =
            SessionState::responder(&session_keys, ad, init.base_key, keys.signed_prekey);
        let plaintext = state.decrypt(&init.message, ad_extra, now_ms, rng)?;
        Ok((self.with_active(state), plaintext))
    }

    /// Encrypt under the active session.
    pub fn encrypt(
        &self,
        plaintext: &[u8],
        ad_extra: &[u8],
    ) -> Result<(Self, RatchetMessage), CryptoError> {
        let mut record = self.clone();
        let state = record
            .active
            .as_mut()
            .ok_or_else(|| CryptoError::NoSession(hex::encode(self.peer)))?;
        let message = state.encrypt(plaintext, ad_extra)?;
        Ok((record, message))
    }

    /// Decrypt with the active session, else the inactive ones newest
    /// first, each on a clone; the session that decrypts is the active one
    /// in the returned record. A message already decrypted is
    /// [`CryptoError::DuplicateMessage`]. `now_ms` ages out skipped keys.
    pub fn decrypt(
        &self,
        message: &RatchetMessage,
        ad_extra: &[u8],
        now_ms: i64,
        rng: &dyn RandomSource,
    ) -> Result<(Self, Vec<u8>), CryptoError> {
        let mut failure = None;
        for (slot, state) in self.states() {
            let mut trial = state.clone();
            match trial.decrypt(message, ad_extra, now_ms, rng) {
                Ok(plaintext) => return Ok((self.promoted(slot, trial), plaintext)),
                Err(CryptoError::DuplicateMessage) => return Err(CryptoError::DuplicateMessage),
                Err(e) => failure = Some(e),
            }
        }
        Err(failure.unwrap_or_else(|| CryptoError::NoSession(hex::encode(self.peer))))
    }

    /// Identity == address, and never a session with ourselves.
    fn check_address(&self, our: &Identity, identity_key: &[u8]) -> Result<(), CryptoError> {
        if identity_key != self.peer {
            return Err(CryptoError::IdentityAddressMismatch {
                address: hex::encode(self.peer),
            });
        }
        if our.public_key_bytes() == self.peer {
            return Err(CryptoError::invalid_key(
                "a session with our own identity".into(),
            ));
        }
        Ok(())
    }

    /// Every session, the active one first; `None` marks the active slot.
    fn states(&self) -> impl Iterator<Item = (Option<usize>, &SessionState)> {
        self.active
            .iter()
            .map(|s| (None, s))
            .chain(self.inactive.iter().enumerate().map(|(i, s)| (Some(i), s)))
    }

    /// This record with `state` active and the previous active archived.
    fn with_active(&self, state: SessionState) -> Self {
        let mut record = self.clone();
        record.activate(state);
        record
    }

    /// This record with the session in `slot` replaced by `state` and
    /// made active.
    fn promoted(&self, slot: Option<usize>, state: SessionState) -> Self {
        let mut record = self.clone();
        match slot {
            None => record.active = Some(state),
            Some(index) => {
                record.inactive.remove(index);
                record.activate(state);
            }
        }
        record
    }

    fn activate(&mut self, state: SessionState) {
        if let Some(previous) = self.active.replace(state) {
            self.inactive.insert(0, previous);
            self.inactive.truncate(ARCHIVED_STATES_MAX_LENGTH);
        }
    }
}

fn pqxdh_error(e: &PqxdhError) -> CryptoError {
    CryptoError::SessionError(format!("PQXDH: {e}"))
}

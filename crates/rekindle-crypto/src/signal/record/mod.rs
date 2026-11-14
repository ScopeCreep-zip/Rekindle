//! `SessionRecord`: every session with one peer device, as a pure value
//! (plan E2.2). Each operation takes `&self` and returns the new record,
//! so the caller stores it in the same transaction as whatever the
//! message changed (plan E2.3), and a failure leaves the stored record as
//! it was.
//!
//! The record is stored under `(peer identity, peer device)`. It holds one
//! active session and up to [`MAX_INACTIVE_SESSIONS`] inactive ones, newest
//! first (Sesame §3). The archive policy is [`policy`]'s (plan E2.5h): an
//! inactive session decrypts in place and is never made active again; a
//! peer's initiation is committed only once its first message decrypts,
//! and placed active or archived by the crossing tiebreak; a record that
//! keeps changing direction between sessions archives its active one.
//!
//! Until the peer acknowledges our session (anything decrypts under it),
//! every message we send on it is a [`PreKeyInit`] carrying the
//! initiation's fields ([`Outgoing::PreKey`], PQXDH rev. 3 §3.3), for up to
//! [`PREKEY_WRAP_MAX_AGE_MS`]. A PreKeyInit whose base key is already a
//! session's is decrypted by that session ([`SessionRecord::receive_known_init`]).
//!
//! Identity == address: the record is for one identity key, and a bundle
//! or init under any other key is refused with
//! [`CryptoError::IdentityAddressMismatch`].

#[cfg(test)]
mod archive_tests;
mod codec;
mod policy;
#[cfg(test)]
mod proptests;
#[cfg(test)]
mod tests;

use policy::LastAct;
pub use policy::{MAX_DIRECTION_CHANGES, MAX_INACTIVE_SESSIONS, PREKEY_WRAP_MAX_AGE_MS};

use rekindle_utils::random::RandomSource;

use crate::bytes::to_32;
use crate::error::CryptoError;
use crate::signal::dr::{RatchetMessage, SessionState};
use crate::signal::pqxdh::agreement::{self, PqPrekeyId, ResponderKeys};
use crate::signal::pqxdh::PqxdhError;
use crate::signal::prekeys::PreKeyBundle;
use crate::Identity;

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

/// A message to send: a ratchet message, or, while the peer has not
/// acknowledged the session, the same message wrapped in the session's
/// PreKeyInit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outgoing {
    Message(RatchetMessage),
    PreKey(PreKeyInit),
}

impl Outgoing {
    /// The ratchet message, wrapped or not.
    #[must_use]
    pub fn message(&self) -> &RatchetMessage {
        match self {
            Self::Message(message) => message,
            Self::PreKey(init) => &init.message,
        }
    }
}

/// The fields of our initiation the peer has not acknowledged: every
/// message on the session carries them until something decrypts under it.
#[derive(Clone)]
pub(super) struct PendingInit {
    pub(super) identity_key: [u8; 32],
    pub(super) signed_prekey_id: u32,
    pub(super) one_time_prekey_id: Option<u32>,
    pub(super) pq_prekey: PqPrekeyId,
    pub(super) kem_ciphertext: Vec<u8>,
    /// When we initiated, for [`PREKEY_WRAP_MAX_AGE_MS`].
    pub(super) created_ms: i64,
}

/// One session and, while the peer has not acknowledged it, our pending
/// initiation.
#[derive(Clone)]
pub(super) struct Session {
    pub(super) state: SessionState,
    pub(super) pending: Option<PendingInit>,
}

/// The sessions with one peer device.
#[derive(Clone)]
pub struct SessionRecord {
    peer: [u8; 32],
    active: Option<Session>,
    /// Newest first, at most [`MAX_INACTIVE_SESSIONS`].
    inactive: Vec<Session>,
    last_act: LastAct,
    direction_changes: u8,
}

impl SessionRecord {
    /// A record for `peer_identity` with no session yet.
    #[must_use]
    pub fn new(peer_identity: [u8; 32]) -> Self {
        Self {
            peer: peer_identity,
            active: None,
            inactive: Vec::new(),
            last_act: LastAct::Unset,
            direction_changes: 0,
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

    /// Whether the active session is established: something decrypted
    /// under it, so it has a receiving chain. A peer's re-initiation over
    /// one is refused unless we asked for it (ruling Q1; plan audit G2).
    /// Our own unanswered init ([`Self::pending_init`]) is not established.
    pub fn is_established(&self) -> bool {
        self.active.as_ref().is_some_and(|s| s.state.has_received())
    }

    /// Whether a session in this record has `base_key`: a PreKeyInit with
    /// it is for that session ([`Self::receive_known_init`]), known before
    /// its prekeys are looked up (a consumed one-time key no longer exists
    /// to look up).
    pub fn has_base_key(&self, base_key: &[u8; 32]) -> bool {
        self.sessions().any(|(_, s)| s.state.base_key() == base_key)
    }

    /// The base key of the active session while it is a PreKeyInit of ours
    /// the peer has not answered (nothing decrypted under it yet): the one
    /// a `PreKeyRejected` may discard (plan E2.4, B6).
    pub fn pending_init(&self) -> Option<&[u8; 32]> {
        self.active
            .as_ref()
            .filter(|s| !s.state.has_received())
            .map(|s| s.state.base_key())
    }

    /// This record without the pending init `base_key` (the peer refused
    /// its prekeys); `None` when it is not the pending init. No session is
    /// active afterwards: an archived one is never made active again, so
    /// the next send opens a new session.
    #[must_use]
    pub fn without_pending(&self, base_key: &[u8; 32]) -> Option<Self> {
        if self.pending_init() != Some(base_key) {
            return None;
        }
        let mut record = self.clone();
        record.active = None;
        Some(record)
    }

    /// Start a session from the peer's bundle and encrypt the first
    /// message under it (the initiator side) at `now_ms`. The new session
    /// is active; the old one, if any, is archived.
    pub fn initiate(
        &self,
        our: &Identity,
        bundle: &PreKeyBundle,
        first_plaintext: &[u8],
        ad_extra: &[u8],
        now_ms: i64,
        rng: &dyn RandomSource,
    ) -> Result<(Self, PreKeyInit), CryptoError> {
        self.check_address(our, &bundle.identity_key)?;
        let agreed = agreement::initiate(our, bundle, rng).map_err(|e| pqxdh_error(&e))?;
        let state = SessionState::initiator(
            &agreed.keys,
            agreed.ad,
            agreed.base_key,
            to_32(&bundle.signed_prekey, "signed prekey")?,
            rng,
        )?;
        let pending = PendingInit {
            identity_key: our.public_key_bytes(),
            signed_prekey_id: bundle.signed_prekey_id,
            one_time_prekey_id: agreed.one_time_prekey_id,
            pq_prekey: agreed.pq_prekey,
            kem_ciphertext: agreed.kem_ciphertext,
            created_ms: now_ms,
        };
        let mut record = self.clone();
        record.on_initiate(Session {
            state,
            pending: Some(pending),
        });
        let Outgoing::PreKey(init) = record.encrypt_active(first_plaintext, ad_extra, now_ms)?
        else {
            return Err(CryptoError::SessionError(
                "a new initiation is not pending".into(),
            ));
        };
        Ok((record, init))
    }

    /// Accept a peer's PreKeyInit for a new session (the responder side):
    /// build a candidate from our secrets and decrypt its first message
    /// without touching the record; only then is it committed, active or
    /// archived by [`policy`]'s tiebreak. So the caller consumes the
    /// one-time prekeys the init names only on `Ok` (X3DH §3.4). An init
    /// for a session we hold is [`Self::receive_known_init`]'s.
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
        let mut record = self.clone();
        let placement = record.placement(&init.base_key);
        record.commit_received(
            Session {
                state,
                pending: None,
            },
            placement,
        );
        Ok((record, plaintext))
    }

    /// A PreKeyInit whose base key is a session's we hold (a retransmission,
    /// or a later message the peer sent before we answered): its message is
    /// decrypted by that session, in place, consuming no prekey. Anything
    /// else is an error, and a message already decrypted is
    /// [`CryptoError::DuplicateMessage`].
    pub fn receive_known_init(
        &self,
        init: &PreKeyInit,
        ad_extra: &[u8],
        now_ms: i64,
        rng: &dyn RandomSource,
    ) -> Result<(Self, Vec<u8>), CryptoError> {
        if init.identity_key != self.peer {
            return Err(CryptoError::IdentityAddressMismatch {
                address: hex::encode(self.peer),
            });
        }
        let (slot, session) = self
            .sessions()
            .find(|(_, s)| s.state.base_key() == &init.base_key)
            .ok_or_else(|| CryptoError::NoSession(hex::encode(self.peer)))?;
        let mut trial = session.state.clone();
        let plaintext = trial.decrypt(&init.message, ad_extra, now_ms, rng)?;
        Ok((self.received_on(slot, trial), plaintext))
    }

    /// Encrypt under the active session at `now_ms`; while the peer has
    /// not acknowledged it the message is wrapped in its PreKeyInit, and
    /// past [`PREKEY_WRAP_MAX_AGE_MS`] unacknowledged it is not sent on
    /// ([`CryptoError::NoSession`]: the next send opens a new session).
    pub fn encrypt(
        &self,
        plaintext: &[u8],
        ad_extra: &[u8],
        now_ms: i64,
    ) -> Result<(Self, Outgoing), CryptoError> {
        let mut record = self.clone();
        let outgoing = record.encrypt_active(plaintext, ad_extra, now_ms)?;
        Ok((record, outgoing))
    }

    /// Decrypt with the active session, else the inactive ones newest
    /// first, each on a clone; the session that decrypts is written back
    /// where it is (an inactive one is never made active). A message
    /// already decrypted is [`CryptoError::DuplicateMessage`]; one no
    /// session decrypts is one error. `now_ms` ages out skipped keys.
    pub fn decrypt(
        &self,
        message: &RatchetMessage,
        ad_extra: &[u8],
        now_ms: i64,
        rng: &dyn RandomSource,
    ) -> Result<(Self, Vec<u8>), CryptoError> {
        if self.active.is_none() && self.inactive.is_empty() {
            return Err(CryptoError::NoSession(hex::encode(self.peer)));
        }
        for (slot, session) in self.sessions() {
            let mut trial = session.state.clone();
            match trial.decrypt(message, ad_extra, now_ms, rng) {
                Ok(plaintext) => return Ok((self.received_on(slot, trial), plaintext)),
                Err(CryptoError::DuplicateMessage) => return Err(CryptoError::DuplicateMessage),
                Err(e) => tracing::debug!(?slot, error = %e, "a session did not decrypt"),
            }
        }
        Err(CryptoError::decryption(
            "no session with the peer decrypts the message".into(),
        ))
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
    fn sessions(&self) -> impl Iterator<Item = (Option<usize>, &Session)> {
        self.active
            .iter()
            .map(|s| (None, s))
            .chain(self.inactive.iter().enumerate().map(|(i, s)| (Some(i), s)))
    }

    /// This record with the session in `slot` replaced by `state`, which
    /// decrypted a message: the peer has our session, so its pending init
    /// is cleared, and the direction counter steps.
    fn received_on(&self, slot: Option<usize>, state: SessionState) -> Self {
        let mut record = self.clone();
        let session = match slot {
            None => record.active.as_mut(),
            Some(index) => record.inactive.get_mut(index),
        }
        .expect("the slot was just read");
        session.state = state;
        session.pending = None;
        match slot {
            None => record.on_receive_active(),
            Some(_) => record.on_receive_inactive(),
        }
        record
    }

    /// Encrypt on the active session, wrapping while it is pending.
    fn encrypt_active(
        &mut self,
        plaintext: &[u8],
        ad_extra: &[u8],
        now_ms: i64,
    ) -> Result<Outgoing, CryptoError> {
        let no_session = || CryptoError::NoSession(hex::encode(self.peer));
        let session = self.active.as_mut().ok_or_else(no_session)?;
        if let Some(pending) = &session.pending {
            if now_ms.saturating_sub(pending.created_ms) > PREKEY_WRAP_MAX_AGE_MS {
                return Err(no_session());
            }
        }
        let message = session.state.encrypt(plaintext, ad_extra)?;
        let outgoing = match &session.pending {
            None => Outgoing::Message(message),
            Some(pending) => Outgoing::PreKey(PreKeyInit {
                identity_key: pending.identity_key,
                base_key: *session.state.base_key(),
                signed_prekey_id: pending.signed_prekey_id,
                one_time_prekey_id: pending.one_time_prekey_id,
                pq_prekey: pending.pq_prekey,
                kem_ciphertext: pending.kem_ciphertext.clone(),
                message,
            }),
        };
        self.on_send();
        Ok(outgoing)
    }
}

fn pqxdh_error(e: &PqxdhError) -> CryptoError {
    CryptoError::SessionError(format!("PQXDH: {e}"))
}

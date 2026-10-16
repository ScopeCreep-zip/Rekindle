//! The sender-key state of community voice channel sessions (plans C7.20,
//! C7.29; RFC 9605 §5.1): shared by the voice and video media paths.
//!
//! Each participant draws its own random media secret per channel session
//! and sends it, sealed, to every other participant ([`super::seal`]);
//! nobody else encrypts under it.
//! One writer per key means two members can never hold different keys
//! under one index, which a shared, rotated channel key did whenever two
//! members rotated at once (every frame then failed AEAD). Signal group
//! calls, MatrixRTC and Jitsi key media this way.
//!
//! A key index is the RFC's two-part KID: `generation = index >> 8`,
//! `step = index & 0xff`.
//!
//! - **Join:** our key ratchets one step at once
//!   (`base_key[i+1] = HKDF(base_key[i], "SFrame 1.0 Ratchet")`, the
//!   previous step erased) and the joiner gets the new step, so it opens
//!   nothing we sealed before it joined; the others derive the step from
//!   the one they hold (RingRTC `group_call.rs` ratchet on join, Jitsi).
//!   Joins while the key is unused coalesce into one step; at step 255 a
//!   join starts a new generation instead, sent to everyone.
//! - **Leave:** a new generation with a fresh secret goes to everyone left
//!   and is used after [`USE_DELAY`], so it reaches them before frames
//!   under it do (RingRTC `MEDIA_SEND_KEY_ROTATION_DELAY_SECS`). At most
//!   one rotation is pending: a leave during the delay marks another,
//!   started when the pending one is applied ([`ChannelSenderKeys::follow_up`]).
//!   No index is ever bound to a second secret.
//! - **Receive:** the last [`KEYS_PER_SENDER`] keys of each sender
//!   (RingRTC `MAX_RECEIVER_STATES_TO_RETAIN`), installed from a sealed
//!   key a sender pushed or derived forward at most [`MAX_RATCHET_AHEAD`]
//!   steps (RingRTC `MAX_OOO_RATCHETS`); a frame under a key we cannot
//!   reach is dropped and the key requested (RFC 9605 §4.4.4).
//!
//! A session's generations start at a random value: a participant who
//! leaves and rejoins starts a new session, and a receiver still holding
//! its old keys must not mistake the new indices for the old ones.

mod received;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::sframe::{self, SframeSender};
use zeroize::Zeroizing;

pub use received::Candidate;
use received::Received;

use super::session::{self, SessionKey};

/// Keys kept per sender.
pub const KEYS_PER_SENDER: usize = 5;
/// How long a key we rotated to waits before we encrypt under it.
pub const USE_DELAY: Duration = Duration::from_secs(5);
/// How many steps past a held key a receiver derives.
pub const MAX_RATCHET_AHEAD: u64 = 5;
/// The last ratchet step of a generation.
const LAST_STEP: u64 = 0xff;

/// The generation of a key index.
#[must_use]
pub fn generation(index: u64) -> u64 {
    index >> 8
}

/// The ratchet step of a key index.
#[must_use]
pub fn step(index: u64) -> u64 {
    index & LAST_STEP
}

/// One of our keys: its index, secret and SFrame sender state (a fresh tag
/// and CTR per key, RFC 9605 §9.1).
#[derive(Clone)]
pub struct OwnKey {
    pub index: u64,
    pub secret: Zeroizing<[u8; 32]>,
    pub sender: Arc<SframeSender>,
}

impl OwnKey {
    /// Step 0 of `generation`, under a fresh secret.
    fn fresh(generation: u64) -> Self {
        Self {
            index: generation << 8,
            secret: super::fresh(),
            sender: Arc::new(SframeSender::fresh()),
        }
    }

    /// The next ratchet step of this key.
    fn ratcheted(&self) -> Self {
        Self {
            index: self.index + 1,
            secret: sframe::ratchet(&self.secret),
            sender: Arc::new(SframeSender::fresh()),
        }
    }
}

/// What a join asks us to send.
pub struct JoinKeys {
    /// To the joiner: the keys it may hold, none sealed before its join.
    pub joiner: Vec<OwnKey>,
    /// To everyone: a new generation, when the join exhausted a generation.
    pub everyone: Option<OwnKey>,
}

/// What a leave asks us to do.
pub enum LeaveRotation {
    /// Send this key, used after [`USE_DELAY`], to everyone left.
    Rotated(OwnKey),
    /// A rotation is pending already: call [`ChannelSenderKeys::follow_up`]
    /// at this instant and send the key it returns to everyone then.
    Deferred(Instant),
}

/// A key we lack: which sender, and the index its frame named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingKey {
    pub sender: String,
    pub index: u64,
}

struct Inner {
    current: OwnKey,
    /// Whether we sealed under `current`: a join ratchets only a used key.
    current_used: bool,
    pending: Option<(OwnKey, Instant)>,
    /// A leave came while a rotation was pending.
    rotate_again: bool,
    /// Per participant, our key index when we admitted it: it is never
    /// given an older key.
    admitted: HashMap<String, u64>,
    received: Received,
    /// Our session key: media keys are sealed to it (plan C7.29 G2).
    session: SessionKey,
    /// Each participant's verified session key and its pseudonym
    /// signature, which a roster relays.
    peer_sessions: HashMap<String, SignedSessionKey>,
}

/// A participant's session key and its pseudonym's signature over it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedSessionKey {
    pub key: Vec<u8>,
    pub signature: Vec<u8>,
}

impl Inner {
    /// Apply a pending rotation whose delay has passed.
    fn apply_due(&mut self, now: Instant) {
        if self.pending.as_ref().is_some_and(|(_, at)| now >= *at) {
            let (key, _) = self.pending.take().expect("checked");
            self.current = key;
            self.current_used = false;
        }
    }

    /// Start a rotation to the next generation, used after [`USE_DELAY`].
    fn start_rotation(&mut self, now: Instant) -> OwnKey {
        let next = match &self.pending {
            Some((key, _)) => generation(key.index),
            None => generation(self.current.index),
        } + 1;
        let key = OwnKey::fresh(next);
        self.pending = Some((key.clone(), now + USE_DELAY));
        key
    }

    /// Our keys from `index` on: the current one and a pending rotation.
    fn keys_from(&self, index: u64) -> Vec<OwnKey> {
        std::iter::once(&self.current)
            .chain(self.pending.as_ref().map(|(k, _)| k))
            .filter(|k| k.index >= index)
            .cloned()
            .collect()
    }
}

/// Our key and the keys we hold from the other participants, for one
/// channel session.
pub struct ChannelSenderKeys {
    inner: parking_lot::Mutex<Inner>,
}

impl Default for ChannelSenderKeys {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelSenderKeys {
    /// A session with a fresh key at a random starting generation.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: parking_lot::Mutex::new(Inner {
                current: OwnKey::fresh(super::random_session_generation()),
                current_used: false,
                pending: None,
                rotate_again: false,
                admitted: HashMap::new(),
                received: Received::default(),
                session: SessionKey::fresh(),
                peer_sessions: HashMap::new(),
            }),
        }
    }

    /// The key we encrypt under now. A rotated key becomes current once its
    /// delay has passed.
    pub fn send_key(&self, now: Instant) -> OwnKey {
        let mut inner = self.inner.lock();
        inner.apply_due(now);
        inner.current_used = true;
        inner.current.clone()
    }

    /// `joiner` joined: ratchet a used key one step, and admit it from
    /// there.
    pub fn on_join(&self, joiner: &str, now: Instant) -> JoinKeys {
        let mut inner = self.inner.lock();
        inner.apply_due(now);
        let mut everyone = None;
        if inner.current_used {
            if step(inner.current.index) < LAST_STEP {
                inner.current = inner.current.ratcheted();
                inner.current_used = false;
            } else if inner.pending.is_none() {
                everyone = Some(inner.start_rotation(now));
            }
        }
        // A key still used here is at its last step: the joiner waits for
        // the pending generation.
        let from = if inner.current_used {
            inner
                .pending
                .as_ref()
                .map_or(inner.current.index + 1, |(k, _)| k.index)
        } else {
            inner.current.index
        };
        inner.admitted.insert(joiner.to_string(), from);
        JoinKeys {
            joiner: inner.keys_from(from),
            everyone,
        }
    }

    /// A participant left: rotate, so it cannot read what follows.
    pub fn on_leave(&self, now: Instant) -> LeaveRotation {
        let mut inner = self.inner.lock();
        inner.apply_due(now);
        if let Some((_, use_at)) = &inner.pending {
            let use_at = *use_at;
            inner.rotate_again = true;
            return LeaveRotation::Deferred(use_at);
        }
        LeaveRotation::Rotated(inner.start_rotation(now))
    }

    /// At a deferred rotation's instant: apply the pending key and, if a
    /// leave came during its delay, start the next rotation (to everyone).
    pub fn follow_up(&self, now: Instant) -> Option<OwnKey> {
        let mut inner = self.inner.lock();
        inner.apply_due(now);
        if !inner.rotate_again || inner.pending.is_some() {
            return None;
        }
        inner.rotate_again = false;
        Some(inner.start_rotation(now))
    }

    /// The keys `participant` may be given on request: ours from its
    /// admission on. Nothing for a participant we have not admitted.
    pub fn shareable_for(&self, participant: &str) -> Vec<OwnKey> {
        let inner = self.inner.lock();
        inner
            .admitted
            .get(participant)
            .map(|from| inner.keys_from(*from))
            .unwrap_or_default()
    }

    /// Whether a delivery of our key at `index` is still worth making: the
    /// pending key, or a step of the current generation the recipient
    /// derives the current step from.
    pub fn still_current(&self, index: u64) -> bool {
        let inner = self.inner.lock();
        inner
            .pending
            .as_ref()
            .is_some_and(|(k, _)| k.index == index)
            || (generation(index) == generation(inner.current.index)
                && index <= inner.current.index
                && inner.current.index - index <= MAX_RATCHET_AHEAD)
    }

    /// Install a key `sender` sent us.
    pub fn install(&self, sender: &str, index: u64, secret: Zeroizing<[u8; 32]>) {
        self.inner.lock().received.install(sender, index, secret);
    }

    /// A candidate opened a frame: keep it.
    pub fn confirm(&self, sender: &str, candidate: Candidate) {
        self.install(sender, candidate.index, candidate.secret);
    }

    /// Forget a participant who left the channel: its keys, admission and
    /// session key.
    pub fn forget(&self, participant: &str) {
        let mut inner = self.inner.lock();
        inner.received.forget(participant);
        inner.admitted.remove(participant);
        inner.peer_sessions.remove(participant);
    }

    /// Our session key's public half, announced in the join handshake.
    pub fn session_public(&self) -> [u8; 32] {
        self.inner.lock().session.public()
    }

    /// Learn `participant`'s session key if its pseudonym signed it for
    /// this channel; a later one (a rejoin) replaces it. Returns whether
    /// it verified.
    pub fn learn_session_key(
        &self,
        community_id: &str,
        channel_id: &str,
        participant: &str,
        signed: &SignedSessionKey,
    ) -> bool {
        let message = session::session_key_message(community_id, channel_id, &signed.key);
        if signed.key.len() != 32
            || !session::verify_pseudonym(participant, &message, &signed.signature)
        {
            return false;
        }
        self.inner
            .lock()
            .peer_sessions
            .insert(participant.to_string(), signed.clone());
        true
    }

    /// `participant`'s verified session key, to relay in a roster.
    pub fn session_key_of(&self, participant: &str) -> Option<SignedSessionKey> {
        self.inner.lock().peer_sessions.get(participant).cloned()
    }

    /// Seal `secret` to `participant`'s session key, or `None` while we do
    /// not hold one.
    pub fn seal_for(&self, participant: &str, aad: &[u8], secret: &[u8; 32]) -> Option<Vec<u8>> {
        let key = self
            .inner
            .lock()
            .peer_sessions
            .get(participant)?
            .key
            .clone();
        session::seal(&key, aad, secret).ok()
    }

    /// Open a media key sealed to our session key.
    ///
    /// # Errors
    /// Not sealed to our session key under this AAD.
    pub fn open_sealed(
        &self,
        aad: &[u8],
        sealed: &[u8],
    ) -> Result<Zeroizing<[u8; 32]>, rekindle_types::error::CryptoError> {
        self.inner.lock().session.open(aad, sealed)
    }

    /// The secret `sender` used under exactly `index` (a video frame
    /// carries its full key index), derived forward and kept if needed.
    ///
    /// # Errors
    /// We hold no key of that sender it can be reached from.
    pub fn secret_at(&self, sender: &str, index: u64) -> Result<Zeroizing<[u8; 32]>, MissingKey> {
        let mut inner = self.inner.lock();
        let secret = inner.received.at(sender, index).ok_or_else(|| MissingKey {
            sender: sender.to_string(),
            index,
        })?;
        inner.received.install(sender, index, secret.clone());
        Ok(secret)
    }

    /// The secrets a voice frame from `sender` whose KID carries
    /// `step_low` may be under, newest generation first. The caller
    /// [`Self::confirm`]s the one that opens it.
    ///
    /// # Errors
    /// None reachable: the index to ask for.
    pub fn receive_candidates(
        &self,
        sender: &str,
        step_low: u8,
    ) -> Result<Vec<Candidate>, MissingKey> {
        self.inner.lock().received.candidates(sender, step_low)
    }
}

/// Our sender keys per community voice channel session.
#[derive(Default)]
pub struct ChannelSenderKeyStore {
    by_channel: parking_lot::Mutex<HashMap<(String, String), Arc<ChannelSenderKeys>>>,
}

impl ChannelSenderKeyStore {
    /// The session's keys, created on first use.
    pub fn keys(&self, community_id: &str, channel_id: &str) -> Arc<ChannelSenderKeys> {
        Arc::clone(
            self.by_channel
                .lock()
                .entry((community_id.to_string(), channel_id.to_string()))
                .or_default(),
        )
    }

    /// End a channel session: its keys go, our session key with them, so
    /// nothing sealed to it opens afterwards (a rejoin draws new ones).
    pub fn end(&self, community_id: &str, channel_id: &str) {
        self.by_channel
            .lock()
            .remove(&(community_id.to_string(), channel_id.to_string()));
    }

    /// Forget every channel (logout).
    pub fn clear(&self) {
        self.by_channel.lock().clear();
    }
}

#[cfg(test)]
mod tests;

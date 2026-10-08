//! The sender-key state of community voice channel sessions (plan C7.20;
//! RFC 9605 §5.1): shared by the voice and video media paths.
//!
//! Each participant draws its own random media secret per channel session
//! and sends it, sealed, to every other participant ([`super::seal`]);
//! nobody else encrypts under it.
//! One writer per key means two members can never hold different keys
//! under one index, which a shared, rotated channel key did whenever two
//! members rotated at once (every frame then failed AEAD). Signal group
//! calls, MatrixRTC and Jitsi key media this way.
//!
//! - **Join:** the joiner gets our current key if it is younger than
//!   [`SHARE_GRACE`]; an older key is rotated first and the new one goes to
//!   everyone (MatrixRTC `RTCEncryptionManager`: "share the existing key
//!   only to new joiners", rotate past `keyRotationGracePeriodMs`).
//! - **Leave:** a fresh key goes to everyone left and is used after
//!   [`USE_DELAY`], so it reaches them before frames under it do (RingRTC
//!   `MEDIA_SEND_KEY_ROTATION_DELAY_SECS`, MSC4143). At most one rotation
//!   is pending; leaves during the delay replace it.
//! - **Receive:** the last [`KEYS_PER_SENDER`] keys of each sender
//!   (RingRTC `MAX_RECEIVER_STATES_TO_RETAIN`), installed only from a
//!   sealed key a sender pushed; a frame under an index we lack is dropped
//!   and its key requested (RFC 9605 §4.4.4).
//!
//! A session's indices start at a random value: a participant who leaves
//! and rejoins starts a new session, and a receiver still holding its old
//! keys must not mistake the new index 0 for the old one.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::sframe::SframeSender;
use zeroize::Zeroizing;

/// Keys kept per sender.
pub const KEYS_PER_SENDER: usize = 5;
/// How long a key we rotated to waits before we encrypt under it.
pub const USE_DELAY: Duration = Duration::from_secs(5);
/// A key younger than this is shared with a joiner as it is.
pub const SHARE_GRACE: Duration = Duration::from_secs(10);

/// One of our keys: its index, secret and SFrame sender state (a fresh tag
/// and CTR per key, RFC 9605 §9.1).
#[derive(Clone)]
pub struct OwnKey {
    pub index: u64,
    pub secret: Zeroizing<[u8; 32]>,
    pub sender: Arc<SframeSender>,
    created: Instant,
}

impl OwnKey {
    fn new(index: u64, now: Instant) -> Self {
        Self {
            index,
            secret: super::fresh(),
            sender: Arc::new(SframeSender::fresh()),
            created: now,
        }
    }
}

/// What a join asks us to send.
pub enum JoinShare {
    /// Our current key, to the joiner only.
    Current(OwnKey),
    /// A key we rotated to (used after [`USE_DELAY`]), to everyone.
    Rotated(OwnKey),
}

/// A key we lack: which sender, and the index its frame named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingKey {
    pub sender: String,
    pub index: u64,
}

struct Inner {
    current: OwnKey,
    pending: Option<(OwnKey, Instant)>,
    received: HashMap<String, VecDeque<(u64, Zeroizing<[u8; 32]>)>>,
}

/// Our key and the keys we hold from the other participants, for one
/// channel session.
pub struct ChannelSenderKeys {
    inner: parking_lot::Mutex<Inner>,
}

impl Default for ChannelSenderKeys {
    fn default() -> Self {
        Self::new(Instant::now())
    }
}

impl ChannelSenderKeys {
    /// A session with a fresh key at a random starting index.
    #[must_use]
    pub fn new(now: Instant) -> Self {
        let start = super::random_session_index();
        Self {
            inner: parking_lot::Mutex::new(Inner {
                current: OwnKey::new(start, now),
                pending: None,
                received: HashMap::new(),
            }),
        }
    }

    /// The key we encrypt under now. A rotated key becomes current once its
    /// delay has passed.
    pub fn send_key(&self, now: Instant) -> OwnKey {
        let mut inner = self.inner.lock();
        if let Some((_, use_at)) = &inner.pending {
            if now >= *use_at {
                let (key, _) = inner.pending.take().expect("checked");
                inner.current = key;
            }
        }
        inner.current.clone()
    }

    /// A participant joined: share our current key, or rotate a stale one.
    pub fn on_join(&self, now: Instant) -> JoinShare {
        let mut inner = self.inner.lock();
        if let Some((key, _)) = &inner.pending {
            // A rotation is already on its way to everyone: the joiner gets
            // that key, which everyone will use.
            return JoinShare::Current(key.clone());
        }
        if now.duration_since(inner.current.created) < SHARE_GRACE {
            return JoinShare::Current(inner.current.clone());
        }
        JoinShare::Rotated(Self::rotate(&mut inner, now))
    }

    /// A participant left: rotate, so it cannot read what follows.
    pub fn on_leave(&self, now: Instant) -> OwnKey {
        Self::rotate(&mut self.inner.lock(), now)
    }

    /// The pending rotation's key: replaced if one is pending (rapid leaves
    /// coalesce into one rotation), else the next index.
    fn rotate(inner: &mut Inner, now: Instant) -> OwnKey {
        let index = match &inner.pending {
            // Replacing the pending key keeps its index: nobody encrypts
            // under it yet, and the replacement is sent to everyone left.
            Some((key, _)) => key.index,
            None => inner.current.index + 1,
        };
        let key = OwnKey::new(index, now);
        inner.pending = Some((key.clone(), now + USE_DELAY));
        key
    }

    /// The keys a participant asking for ours needs: the current one and a
    /// pending rotation.
    pub fn shareable(&self) -> Vec<OwnKey> {
        let inner = self.inner.lock();
        std::iter::once(inner.current.clone())
            .chain(inner.pending.as_ref().map(|(k, _)| k.clone()))
            .collect()
    }

    /// Install a key `sender` sent us. The same index from a sender
    /// replaces the old secret; the oldest is forgotten past
    /// [`KEYS_PER_SENDER`].
    pub fn install(&self, sender: &str, index: u64, secret: Zeroizing<[u8; 32]>) {
        let mut inner = self.inner.lock();
        let keys = inner.received.entry(sender.to_string()).or_default();
        keys.retain(|(i, _)| *i != index);
        keys.push_back((index, secret));
        while keys.len() > KEYS_PER_SENDER {
            keys.pop_front();
        }
    }

    /// Forget a sender who left the channel.
    pub fn forget(&self, sender: &str) {
        self.inner.lock().received.remove(sender);
    }

    /// The secret `sender` sent us under exactly `index` (a video frame
    /// carries its full key index).
    ///
    /// # Errors
    /// We hold no key of that sender under that index.
    pub fn secret_at(&self, sender: &str, index: u64) -> Result<Zeroizing<[u8; 32]>, MissingKey> {
        self.inner
            .lock()
            .received
            .get(sender)
            .and_then(|keys| keys.iter().rev().find(|(i, _)| *i == index))
            .map(|(_, secret)| secret.clone())
            .ok_or_else(|| MissingKey {
                sender: sender.to_string(),
                index,
            })
    }

    /// The secret `sender` used for a frame whose KID carries `index_low`,
    /// the low 8 bits of its key index.
    ///
    /// # Errors
    /// We hold no key of that sender under that index: the index to ask for
    /// (the next one with those low bits past the newest we hold).
    pub fn receive_secret(
        &self,
        sender: &str,
        index_low: u8,
    ) -> Result<Zeroizing<[u8; 32]>, MissingKey> {
        let inner = self.inner.lock();
        let keys = inner.received.get(sender);
        if let Some((_, secret)) = keys.and_then(|keys| {
            keys.iter()
                .rev()
                .find(|(i, _)| (*i & 0xff) as u8 == index_low)
        }) {
            return Ok(secret.clone());
        }
        let newest = keys.and_then(|keys| keys.iter().map(|(i, _)| *i).max());
        let index = match newest {
            Some(newest) => {
                let candidate = (newest & !0xff) | u64::from(index_low);
                if candidate > newest {
                    candidate
                } else {
                    candidate.saturating_add(0x100)
                }
            }
            None => u64::from(index_low),
        };
        Err(MissingKey {
            sender: sender.to_string(),
            index,
        })
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

    /// End a channel session: its keys go (a rejoin draws new ones).
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

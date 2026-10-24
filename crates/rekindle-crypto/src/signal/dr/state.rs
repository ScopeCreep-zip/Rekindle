//! One session's Double Ratchet state and its steps (DR rev. 4 §3.3
//! initialization, §3.4 encrypt, §3.5 decrypt).
//!
//! Every mutating step runs on a value the caller owns; the session
//! record clones before decrypting and keeps the clone only on success,
//! which is §3.5's rule that "changes to the state object are discarded"
//! on an exception. The steps themselves also refuse before they mutate
//! wherever a refusal can be known first: the forward-jump bounds, the
//! message number, and the DH ratchet's agreements.

use rekindle_utils::random::RandomSource;
use x25519_dalek::{PublicKey as X25519Public, StaticSecret};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use super::kdf::{kdf_ck, kdf_rk, open, seal};
use super::{
    Header, RatchetMessage, MAX_RECEIVING_CHAINS_KEPT, MAX_SKIP, MAX_SKIPPED_KEYS_PER_CHAIN,
    SKIPPED_KEY_MAX_AGE_MS,
};
use crate::error::CryptoError;
use crate::signal::pqxdh::agreement::{AssociatedData, SessionKeys};

/// A chain key and the number of the next message on it (`CKs`/`Ns` or
/// `CKr`/`Nr`).
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub(super) struct Chain {
    pub(super) key: [u8; 32],
    pub(super) n: u32,
}

/// A message key kept for a message not yet received (`MKSKIPPED`), and
/// when it was stored (DR rev. 4 §8.4: delete skipped keys after an
/// interval).
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub(super) struct SkippedKey {
    pub(super) n: u32,
    pub(super) stored_at_ms: i64,
    pub(super) key: [u8; 32],
}

/// A DH ratchet step computed but not yet applied: both agreements ran,
/// so applying it cannot fail.
struct RatchetStep {
    their: [u8; 32],
    receive_key: Zeroizing<[u8; 32]>,
    root_key: Zeroizing<[u8; 32]>,
    send_key: Zeroizing<[u8; 32]>,
    dh_self: Zeroizing<[u8; 32]>,
}

/// A receiving chain: the sender's ratchet key it answers, its chain and
/// the keys skipped on it, oldest first.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub(super) struct ReceiverChain {
    pub(super) their: [u8; 32],
    pub(super) chain: Chain,
    pub(super) skipped: Vec<SkippedKey>,
}

/// §3.2's state, plus what the session record needs to know about it.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub(crate) struct SessionState {
    /// `EncodeEC(IK_A) ‖ EncodeEC(IK_B)`, bound into every message.
    pub(super) ad: AssociatedData,
    /// `EK_A`'s public half: the session's identity for Sesame's
    /// base-key idempotency.
    pub(super) base_key: [u8; 32],
    /// `SKscka`, kept for the post-quantum ratchet (F2).
    pub(super) sk_scka: [u8; 32],
    /// `RK`.
    pub(super) root_key: [u8; 32],
    /// `DHs`, the secret half.
    pub(super) dh_self: [u8; 32],
    /// `DHr`.
    pub(super) remote: Option<[u8; 32]>,
    /// `CKs` and `Ns`; `None` until the responder's first DH ratchet.
    pub(super) sender: Option<Chain>,
    /// `PN`.
    pub(super) pn: u32,
    /// The receiving chains, oldest first; the last answers `DHr`.
    pub(super) receivers: Vec<ReceiverChain>,
}

impl SessionState {
    /// `RatchetInitAlice`: a fresh `DHs` from `rng`, `DHr` = the
    /// responder's signed prekey, and the first sending chain.
    pub(crate) fn initiator(
        keys: &SessionKeys,
        ad: AssociatedData,
        base_key: [u8; 32],
        their_signed_prekey: [u8; 32],
        rng: &dyn RandomSource,
    ) -> Result<Self, CryptoError> {
        let dh_self = generate_dh(rng);
        let dh_out = agree(&dh_self, &their_signed_prekey)?;
        let (root_key, chain_key) = kdf_rk(&keys.sk_ec, &dh_out);
        Ok(Self {
            ad,
            base_key,
            sk_scka: *keys.sk_scka,
            root_key: *root_key,
            dh_self: *dh_self,
            remote: Some(their_signed_prekey),
            sender: Some(Chain {
                key: *chain_key,
                n: 0,
            }),
            pn: 0,
            receivers: Vec::new(),
        })
    }

    /// `RatchetInitBob`: `DHs` = our signed prekey, `RK` = `SKec`, no
    /// chains until the initiator's first message.
    pub(crate) fn responder(
        keys: &SessionKeys,
        ad: AssociatedData,
        base_key: [u8; 32],
        our_signed_prekey: &StaticSecret,
    ) -> Self {
        Self {
            ad,
            base_key,
            sk_scka: *keys.sk_scka,
            root_key: *keys.sk_ec,
            dh_self: our_signed_prekey.to_bytes(),
            remote: None,
            sender: None,
            pn: 0,
            receivers: Vec::new(),
        }
    }

    /// The session's base key.
    pub(crate) fn base_key(&self) -> &[u8; 32] {
        &self.base_key
    }

    /// Whether anything has decrypted under this session: an initiator's
    /// session has no receiving chain until the peer's first reply.
    pub(crate) fn has_received(&self) -> bool {
        !self.receivers.is_empty()
    }

    /// `RatchetEncrypt`.
    pub(crate) fn encrypt(
        &mut self,
        plaintext: &[u8],
        ad_extra: &[u8],
    ) -> Result<RatchetMessage, CryptoError> {
        let dh = X25519Public::from(&StaticSecret::from(self.dh_self)).to_bytes();
        let sender = self
            .sender
            .as_mut()
            .ok_or_else(|| CryptoError::SessionError("no sending chain yet".into()))?;
        let header = Header {
            dh,
            pn: self.pn,
            n: sender.n,
        };
        let (message_key, next) = kdf_ck(&sender.key);
        sender.key = *next;
        sender.n = sender
            .n
            .checked_add(1)
            .ok_or_else(|| CryptoError::SessionError("sending chain exhausted".into()))?;
        let ciphertext = seal(&message_key, plaintext, &self.message_ad(ad_extra, &header));
        Ok(RatchetMessage { header, ciphertext })
    }

    /// `RatchetDecrypt` at `now_ms`, with duplicate detection:
    /// a message whose key was already used is
    /// [`CryptoError::DuplicateMessage`]. Skipped keys older than
    /// [`SKIPPED_KEY_MAX_AGE_MS`] are deleted first.
    pub(crate) fn decrypt(
        &mut self,
        message: &RatchetMessage,
        ad_extra: &[u8],
        now_ms: i64,
        rng: &dyn RandomSource,
    ) -> Result<Vec<u8>, CryptoError> {
        let header = &message.header;
        // Message u32::MAX would leave its chain's next number at 2^32.
        if header.n == u32::MAX {
            return Err(CryptoError::decryption(
                "message number past the chain's end".into(),
            ));
        }
        let ad = self.message_ad(ad_extra, header);
        self.expire_skipped(now_ms);
        if let Some(key) = self.take_skipped(header) {
            return open(&key, &message.ciphertext, &ad);
        }
        if self.remote == Some(header.dh) {
            if self.current().is_some_and(|c| header.n < c.chain.n) {
                return Err(CryptoError::DuplicateMessage);
            }
        } else {
            if let Some(old) = self.receivers.iter().find(|c| c.their == header.dh) {
                return Err(if header.n < old.chain.n {
                    CryptoError::DuplicateMessage
                } else {
                    CryptoError::decryption("message on a retired chain".into())
                });
            }
            // A new chain starts at 0. Both bounds hold before any key
            // (or the ratchet's agreement) is derived.
            // Then the ratchet's agreements, so a low-order key is refused
            // before anything is stored.
            check_forward_jump(header.pn, self.current().map_or(0, |c| c.chain.n))?;
            check_forward_jump(header.n, 0)?;
            let step = self.ratchet_step(header.dh, rng)?;
            self.skip_message_keys(header.pn, now_ms)?;
            self.apply_ratchet(&step);
        }
        self.skip_message_keys(header.n, now_ms)?;
        let chain = &mut self
            .current_mut()
            .ok_or_else(|| CryptoError::decryption("no receiving chain".into()))?
            .chain;
        let (message_key, next) = kdf_ck(&chain.key);
        chain.key = *next;
        chain.n = chain
            .n
            .checked_add(1)
            .ok_or_else(|| CryptoError::decryption("receiving chain exhausted".into()))?;
        open(&message_key, &message.ciphertext, &ad)
    }

    /// `AD ‖ ad_extra ‖ header`.
    fn message_ad(&self, ad_extra: &[u8], header: &Header) -> Vec<u8> {
        let header = header.to_bytes();
        let mut ad = Vec::with_capacity(self.ad.len() + ad_extra.len() + header.len());
        ad.extend_from_slice(&self.ad);
        ad.extend_from_slice(ad_extra);
        ad.extend_from_slice(&header);
        ad
    }

    /// Delete every skipped key stored more than
    /// [`SKIPPED_KEY_MAX_AGE_MS`] before `now_ms` (DR rev. 4 §8.4). A key
    /// "stored" in the future (the clock went back) is kept.
    fn expire_skipped(&mut self, now_ms: i64) {
        for receiver in &mut self.receivers {
            receiver.skipped.retain(|k| {
                u64::try_from(now_ms.saturating_sub(k.stored_at_ms))
                    .map_or(true, |age| age <= SKIPPED_KEY_MAX_AGE_MS)
            });
        }
    }

    /// `TrySkippedMessageKeys`: remove and return the key for `(dh, n)`.
    fn take_skipped(&mut self, header: &Header) -> Option<Zeroizing<[u8; 32]>> {
        let chain = self.receivers.iter_mut().find(|c| c.their == header.dh)?;
        let at = chain.skipped.iter().position(|k| k.n == header.n)?;
        Some(Zeroizing::new(chain.skipped.remove(at).key))
    }

    /// The receiving chain for `DHr`, if one exists yet.
    fn current(&self) -> Option<&ReceiverChain> {
        self.receivers
            .last()
            .filter(|c| Some(c.their) == self.remote)
    }

    fn current_mut(&mut self) -> Option<&mut ReceiverChain> {
        let remote = self.remote;
        self.receivers
            .last_mut()
            .filter(|c| Some(c.their) == remote)
    }

    /// `SkipMessageKeys(until)`: keep the keys up to `until` on the
    /// current receiving chain, stamped `now_ms`, refusing a jump past
    /// [`MAX_SKIP`] and keeping the newest [`MAX_SKIPPED_KEYS_PER_CHAIN`].
    fn skip_message_keys(&mut self, until: u32, now_ms: i64) -> Result<(), CryptoError> {
        check_forward_jump(until, self.current().map_or(0, |c| c.chain.n))?;
        let Some(current) = self.current_mut() else {
            return Ok(());
        };
        while current.chain.n < until {
            let (message_key, next) = kdf_ck(&current.chain.key);
            current.skipped.push(SkippedKey {
                n: current.chain.n,
                stored_at_ms: now_ms,
                key: *message_key,
            });
            current.chain.key = *next;
            // `n < until`, so this never passes u32::MAX.
            current.chain.n += 1;
        }
        let excess = current
            .skipped
            .len()
            .saturating_sub(MAX_SKIPPED_KEYS_PER_CHAIN);
        current.skipped.drain(..excess);
        Ok(())
    }

    /// `DHRatchet`'s derivations for the sender's new key `their`: the
    /// receiving chain, a fresh `DHs` from `rng`, the sending chain and
    /// the new root. Nothing is changed, so a non-contributory agreement
    /// (a low-order key) refuses the message with the state untouched.
    fn ratchet_step(
        &self,
        their: [u8; 32],
        rng: &dyn RandomSource,
    ) -> Result<RatchetStep, CryptoError> {
        let receive_dh = agree(&self.dh_self, &their)?;
        let (root_key, receive_key) = kdf_rk(&self.root_key, &receive_dh);
        let dh_self = generate_dh(rng);
        let send_dh = agree(&dh_self, &their)?;
        let (root_key, send_key) = kdf_rk(&root_key, &send_dh);
        Ok(RatchetStep {
            their,
            receive_key,
            root_key,
            send_key,
            dh_self,
        })
    }

    /// `DHRatchet`'s state changes from a computed step. Beyond
    /// [`MAX_RECEIVING_CHAINS_KEPT`] the oldest receiving chain goes.
    fn apply_ratchet(&mut self, step: &RatchetStep) {
        self.pn = self.sender.as_ref().map_or(0, |s| s.n);
        self.remote = Some(step.their);
        self.receivers.push(ReceiverChain {
            their: step.their,
            chain: Chain {
                key: *step.receive_key,
                n: 0,
            },
            skipped: Vec::new(),
        });
        let excess = self
            .receivers
            .len()
            .saturating_sub(MAX_RECEIVING_CHAINS_KEPT);
        self.receivers.drain(..excess);
        self.root_key = *step.root_key;
        self.dh_self = *step.dh_self;
        self.sender = Some(Chain {
            key: *step.send_key,
            n: 0,
        });
    }
}

/// Refuse message `until` when it runs more than [`MAX_SKIP`]
/// ahead of a chain whose next message is `next`.
fn check_forward_jump(until: u32, next: u32) -> Result<(), CryptoError> {
    if until.saturating_sub(next) > MAX_SKIP {
        return Err(CryptoError::decryption(format!(
            "message {until} is more than {MAX_SKIP} ahead of its chain"
        )));
    }
    Ok(())
}

/// `GENERATE_DH`: a new X25519 secret from `rng`.
fn generate_dh(rng: &dyn RandomSource) -> Zeroizing<[u8; 32]> {
    let mut secret = Zeroizing::new([0u8; 32]);
    rng.fill(&mut *secret);
    secret
}

/// `DH(our, their)`, refusing a non-contributory result (a low-order
/// public key from the peer).
fn agree(our: &[u8; 32], their: &[u8; 32]) -> Result<Zeroizing<[u8; 32]>, CryptoError> {
    let shared = StaticSecret::from(*our).diffie_hellman(&X25519Public::from(*their));
    if !shared.was_contributory() {
        return Err(CryptoError::decryption(
            "non-contributory ratchet agreement".into(),
        ));
    }
    Ok(Zeroizing::new(shared.to_bytes()))
}

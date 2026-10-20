//! One session's Double Ratchet state and its §3.5 steps.
//!
//! Every mutating step runs on a value the caller owns; the session
//! record clones before decrypting and keeps the clone only on success,
//! which is §3.5's rule that a failed decryption changes nothing.

use rekindle_utils::random::RandomSource;
use x25519_dalek::{PublicKey as X25519Public, StaticSecret};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use super::kdf::{kdf_ck, kdf_rk, open, seal};
use super::{Header, RatchetMessage, MAX_FORWARD_JUMPS, MAX_MESSAGE_KEYS, MAX_RECEIVER_CHAINS};
use crate::error::CryptoError;
use crate::signal::pqxdh::agreement::{AssociatedData, SessionKeys};

/// A chain key and the number of the next message on it (`CKs`/`Ns` or
/// `CKr`/`Nr`).
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub(super) struct Chain {
    pub(super) key: [u8; 32],
    pub(super) n: u32,
}

/// A message key kept for a message not yet received (`MKSKIPPED`).
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub(super) struct SkippedKey {
    pub(super) n: u32,
    pub(super) key: [u8; 32],
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
        let ciphertext = seal(&message_key, plaintext, &self.message_ad(ad_extra, &header))?;
        Ok(RatchetMessage { header, ciphertext })
    }

    /// `RatchetDecrypt`, with libsignal's duplicate detection: a message
    /// whose key was already used is [`CryptoError::DuplicateMessage`].
    pub(crate) fn decrypt(
        &mut self,
        message: &RatchetMessage,
        ad_extra: &[u8],
        rng: &dyn RandomSource,
    ) -> Result<Vec<u8>, CryptoError> {
        let header = &message.header;
        let ad = self.message_ad(ad_extra, header);
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
            check_forward_jump(header.pn, self.current().map_or(0, |c| c.chain.n))?;
            check_forward_jump(header.n, 0)?;
            self.skip_message_keys(header.pn)?;
            self.dh_ratchet(header.dh, rng)?;
        }
        self.skip_message_keys(header.n)?;
        let chain = &mut self
            .current_mut()
            .ok_or_else(|| CryptoError::decryption("no receiving chain".into()))?
            .chain;
        let (message_key, next) = kdf_ck(&chain.key);
        chain.key = *next;
        chain.n += 1;
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
    /// current receiving chain, refusing a jump past
    /// [`MAX_FORWARD_JUMPS`] and keeping the newest [`MAX_MESSAGE_KEYS`].
    fn skip_message_keys(&mut self, until: u32) -> Result<(), CryptoError> {
        check_forward_jump(until, self.current().map_or(0, |c| c.chain.n))?;
        let Some(current) = self.current_mut() else {
            return Ok(());
        };
        while current.chain.n < until {
            let (message_key, next) = kdf_ck(&current.chain.key);
            current.skipped.push(SkippedKey {
                n: current.chain.n,
                key: *message_key,
            });
            current.chain.key = *next;
            current.chain.n += 1;
        }
        let excess = current.skipped.len().saturating_sub(MAX_MESSAGE_KEYS);
        current.skipped.drain(..excess);
        Ok(())
    }

    /// `DHRatchet`: a receiving chain for the sender's new key, then a
    /// fresh `DHs` from `rng` and a sending chain. Beyond
    /// [`MAX_RECEIVER_CHAINS`] the oldest receiving chain goes.
    fn dh_ratchet(&mut self, their: [u8; 32], rng: &dyn RandomSource) -> Result<(), CryptoError> {
        self.pn = self.sender.as_ref().map_or(0, |s| s.n);
        self.remote = Some(their);
        let receive_dh = agree(&self.dh_self, &their)?;
        let (root_key, receive_key) = kdf_rk(&self.root_key, &receive_dh);
        self.receivers.push(ReceiverChain {
            their,
            chain: Chain {
                key: *receive_key,
                n: 0,
            },
            skipped: Vec::new(),
        });
        let excess = self.receivers.len().saturating_sub(MAX_RECEIVER_CHAINS);
        self.receivers.drain(..excess);

        let dh_self = generate_dh(rng);
        let send_dh = agree(&dh_self, &their)?;
        let (root_key, send_key) = kdf_rk(&root_key, &send_dh);
        self.root_key = *root_key;
        self.dh_self = *dh_self;
        self.sender = Some(Chain {
            key: *send_key,
            n: 0,
        });
        Ok(())
    }
}

/// Refuse message `until` when it runs more than [`MAX_FORWARD_JUMPS`]
/// ahead of a chain whose next message is `next`.
fn check_forward_jump(until: u32, next: u32) -> Result<(), CryptoError> {
    if until.saturating_sub(next) > MAX_FORWARD_JUMPS {
        return Err(CryptoError::decryption(format!(
            "message {until} is more than {MAX_FORWARD_JUMPS} ahead of its chain"
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

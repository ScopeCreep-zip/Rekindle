//! The reference model: DR rev. 4 §3.5's pseudocode, line for line, over
//! the same §7.2 functions, so a difference from [`SessionState`] is a
//! difference in the state machine.
//!
//! Two additions, both from the spec's own text: a decryption that fails
//! leaves the state as it was (§3.5, "changes to the state object are
//! discarded"), and skipped keys are deleted by the policy §3.2 leaves to
//! the implementation — here the bounds of §8.4: the keys of the newest
//! [`MAX_RECEIVING_CHAINS_KEPT`] receiving keys only, at most
//! [`MAX_SKIPPED_KEYS_PER_CHAIN`] per receiving key, oldest first.
//!
//! [`SessionState`]: super::super::SessionState

use rekindle_utils::random::RandomSource;
use x25519_dalek::{PublicKey as X25519Public, StaticSecret};

use super::super::kdf::{kdf_ck, kdf_rk, open, seal};
use super::super::{
    Header, RatchetMessage, MAX_RECEIVING_CHAINS_KEPT, MAX_SKIP, MAX_SKIPPED_KEYS_PER_CHAIN,
};
use crate::error::CryptoError;

#[derive(Clone)]
pub(super) struct Model {
    ad: Vec<u8>,
    dhs: [u8; 32],
    dhr: Option<[u8; 32]>,
    rk: [u8; 32],
    cks: Option<[u8; 32]>,
    ckr: Option<[u8; 32]>,
    ns: u32,
    nr: u32,
    pn: u32,
    /// `MKSKIPPED`, in insertion order.
    mkskipped: Vec<(([u8; 32], u32), [u8; 32])>,
    /// The receiving keys `DHr` has held, oldest first (the deletion
    /// policy's input).
    history: Vec<[u8; 32]>,
}

fn generate_dh(rng: &dyn RandomSource) -> [u8; 32] {
    let mut secret = [0u8; 32];
    rng.fill(&mut secret);
    secret
}

fn dh(secret: &[u8; 32], public: &[u8; 32]) -> [u8; 32] {
    StaticSecret::from(*secret)
        .diffie_hellman(&X25519Public::from(*public))
        .to_bytes()
}

fn public(secret: &[u8; 32]) -> [u8; 32] {
    X25519Public::from(&StaticSecret::from(*secret)).to_bytes()
}

impl Model {
    /// `RatchetInitAlice`.
    pub(super) fn alice(
        sk: &[u8; 32],
        ad: &[u8],
        bob_dh_public: [u8; 32],
        rng: &dyn RandomSource,
    ) -> Self {
        let dhs = generate_dh(rng);
        let (rk, cks) = kdf_rk(sk, &dh(&dhs, &bob_dh_public));
        Self {
            ad: ad.to_vec(),
            dhs,
            dhr: Some(bob_dh_public),
            rk: *rk,
            cks: Some(*cks),
            ckr: None,
            ns: 0,
            nr: 0,
            pn: 0,
            mkskipped: Vec::new(),
            history: Vec::new(),
        }
    }

    /// `RatchetInitBob`.
    pub(super) fn bob(sk: &[u8; 32], ad: &[u8], bob_dh_secret: [u8; 32]) -> Self {
        Self {
            ad: ad.to_vec(),
            dhs: bob_dh_secret,
            dhr: None,
            rk: *sk,
            cks: None,
            ckr: None,
            ns: 0,
            nr: 0,
            pn: 0,
            mkskipped: Vec::new(),
            history: Vec::new(),
        }
    }

    /// `RatchetEncrypt`; `None` while `CKs` is `None`.
    pub(super) fn encrypt(&mut self, plaintext: &[u8], ad_extra: &[u8]) -> Option<RatchetMessage> {
        let (mk, cks) = kdf_ck(&self.cks?);
        self.cks = Some(*cks);
        let header = Header {
            dh: public(&self.dhs),
            pn: self.pn,
            n: self.ns,
        };
        self.ns += 1;
        let ciphertext = seal(&mk, plaintext, &self.concat(ad_extra, &header));
        Some(RatchetMessage { header, ciphertext })
    }

    /// `RatchetDecrypt`; on failure the state is restored.
    pub(super) fn decrypt(
        &mut self,
        message: &RatchetMessage,
        ad_extra: &[u8],
        rng: &dyn RandomSource,
    ) -> Option<Vec<u8>> {
        let saved = self.clone();
        let result = self.try_decrypt(message, ad_extra, rng);
        if result.is_none() {
            *self = saved;
        }
        result
    }

    fn try_decrypt(
        &mut self,
        message: &RatchetMessage,
        ad_extra: &[u8],
        rng: &dyn RandomSource,
    ) -> Option<Vec<u8>> {
        let header = message.header;
        let ad = self.concat(ad_extra, &header);
        if let Some(result) = self.try_skipped_message_keys(&header, &message.ciphertext, &ad) {
            return result.ok();
        }
        if Some(header.dh) != self.dhr {
            self.skip_message_keys(header.pn)?;
            self.dh_ratchet(&header, rng);
        }
        self.skip_message_keys(header.n)?;
        let (mk, ckr) = kdf_ck(&self.ckr?);
        self.ckr = Some(*ckr);
        self.nr += 1;
        open(&mk, &message.ciphertext, &ad).ok()
    }

    fn concat(&self, ad_extra: &[u8], header: &Header) -> Vec<u8> {
        [self.ad.as_slice(), ad_extra, &header.to_bytes()].concat()
    }

    fn try_skipped_message_keys(
        &mut self,
        header: &Header,
        ciphertext: &[u8],
        ad: &[u8],
    ) -> Option<Result<Vec<u8>, CryptoError>> {
        let at = self
            .mkskipped
            .iter()
            .position(|(key, _)| *key == (header.dh, header.n))?;
        let (_, mk) = self.mkskipped.remove(at);
        Some(open(&mk, ciphertext, ad))
    }

    fn skip_message_keys(&mut self, until: u32) -> Option<()> {
        if self.nr.saturating_add(MAX_SKIP) < until {
            return None;
        }
        if let (Some(mut ckr), Some(dhr)) = (self.ckr, self.dhr) {
            while self.nr < until {
                let (mk, next) = kdf_ck(&ckr);
                ckr = *next;
                self.mkskipped.push(((dhr, self.nr), *mk));
                self.nr += 1;
            }
            self.ckr = Some(ckr);
            self.delete_policy();
        }
        Some(())
    }

    fn dh_ratchet(&mut self, header: &Header, rng: &dyn RandomSource) {
        self.pn = self.ns;
        self.ns = 0;
        self.nr = 0;
        self.dhr = Some(header.dh);
        let (rk, ckr) = kdf_rk(&self.rk, &dh(&self.dhs, &header.dh));
        self.ckr = Some(*ckr);
        self.dhs = generate_dh(rng);
        let (rk, cks) = kdf_rk(&rk, &dh(&self.dhs, &header.dh));
        self.rk = *rk;
        self.cks = Some(*cks);
        self.history.push(header.dh);
        self.delete_policy();
    }

    /// Keep the skipped keys of the newest receiving keys only, and the
    /// newest keys of each.
    fn delete_policy(&mut self) {
        let excess = self.history.len().saturating_sub(MAX_RECEIVING_CHAINS_KEPT);
        self.history.drain(..excess);
        let history = &self.history;
        self.mkskipped.retain(|((dh, _), _)| history.contains(dh));
        for dh in history.clone() {
            let held = self.mkskipped.iter().filter(|((d, _), _)| *d == dh).count();
            let mut drop = held.saturating_sub(MAX_SKIPPED_KEYS_PER_CHAIN);
            self.mkskipped.retain(|((d, _), _)| {
                if *d == dh && drop > 0 {
                    drop -= 1;
                    false
                } else {
                    true
                }
            });
        }
    }
}

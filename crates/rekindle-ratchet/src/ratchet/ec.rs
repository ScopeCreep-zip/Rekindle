//! Header-Encrypted Double Ratchet (HE-DR).
//!
//! Implements Signal Double Ratchet spec Revision 4 (2025-11-04) §4.
//!
//! Key design decisions:
//! - `PrivateKey` (reusable) for ratchet DH keys, not `EphemeralPrivateKey`
//! - Header AEAD nonce = counter-based `[0u32 || n_send || 0u32]` (not all-zero)
//! - Skipped keys stored via caller callback, not embedded in session state
//! - Cremers 2023 promotion guard: sessions decrypted from cold storage must
//!   complete a receipt-ack before promotion to active

use zeroize::Zeroizing;

use crate::crypto::{aead, dh, kdf};
use crate::error::RatchetError;
use crate::session::DoubleRatchetState;
use crate::session::skipped::MAX_SKIP_PER_CHAIN;

/// Plaintext header: sender's DH public key + chain metadata.
#[derive(Debug, Clone)]
pub struct MessageHeader {
    pub dh_pub: [u8; 32],
    pub pn: u32,
    pub n: u32,
}

const HEADER_LEN: usize = 40;

impl MessageHeader {
    pub fn to_bytes(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        out[..32].copy_from_slice(&self.dh_pub);
        out[32..36].copy_from_slice(&self.pn.to_be_bytes());
        out[36..40].copy_from_slice(&self.n.to_be_bytes());
        out
    }

    pub fn from_bytes(bytes: &[u8; HEADER_LEN]) -> Self {
        let mut dh_pub = [0u8; 32];
        dh_pub.copy_from_slice(&bytes[..32]);
        let pn = u32::from_be_bytes(bytes[32..36].try_into().expect("4 bytes"));
        let n = u32::from_be_bytes(bytes[36..40].try_into().expect("4 bytes"));
        Self { dh_pub, pn, n }
    }
}

/// Output of `encrypt_he`.
pub struct EncryptedMessage {
    pub encrypted_header: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

/// Callback for storing/retrieving skipped message keys.
pub trait SkippedKeyCallback {
    fn store_skipped(
        &self,
        header_key: &[u8; 32],
        counter: u32,
        message_key: &Zeroizing<[u8; 32]>,
    ) -> Result<(), RatchetError>;

    fn take_skipped(
        &self,
        header_key: &[u8; 32],
        counter: u32,
    ) -> Result<Option<Zeroizing<[u8; 32]>>, RatchetError>;
}

// ── Header decrypt ─────────────────────────────────────────────────

/// Try to decrypt a header with a given header key and counter.
pub(crate) fn try_decrypt_header_at(
    hk: &Zeroizing<[u8; 32]>,
    counter: u32,
    encrypted_header: &[u8],
) -> Option<MessageHeader> {
    if encrypted_header.len() != HEADER_LEN + aead::TAG_LEN {
        return None;
    }
    let hk_key = aead::build_key(hk).ok()?;
    let mut buf = encrypted_header.to_vec();
    let plain = aead::open(&hk_key, counter, &[], &mut buf).ok()?;
    if plain.len() != HEADER_LEN {
        return None;
    }
    let header_bytes: [u8; HEADER_LEN] = plain.try_into().ok()?;
    Some(MessageHeader::from_bytes(&header_bytes))
}

// ── DH ratchet step (shared between ec and triple decrypt paths) ───

/// Perform a DH ratchet step on the receiving side.
///
/// 1. Generates a new DH keypair
/// 2. DH with their new ratchet public → receiving chain
/// 3. DH with their public again → sending chain
/// 4. Updates root key, chain keys, header keys, DH state
///
/// Used by both `decrypt_he` (standalone EC) and `ec_decrypt_split`
/// (Triple Ratchet) to avoid code duplication.
pub(crate) fn perform_dh_ratchet(
    state: &mut DoubleRatchetState,
    their_dh_pub: &[u8; 32],
    skipped: &dyn SkippedKeyCallback,
) -> Result<(), RatchetError> {
    // Skip remaining keys on old receiving chain
    if let (Some(ckr), Some(hkr)) = (&state.ckr, &state.hkr) {
        let mut ck = ckr.clone();
        while state.n_recv < state.pn && state.n_recv < MAX_SKIP_PER_CHAIN {
            let (next, skip_mk) = kdf::kdf_ck(&ck);
            skipped.store_skipped(hkr, state.n_recv, &skip_mk)?;
            ck = next;
            state.n_recv += 1;
        }
    }

    // Promote NHK → HK, reset counters
    state.hks = Some(state.nhks.clone());
    state.hkr = Some(state.nhkr.clone());
    state.pn = state.n_send;
    state.n_send = 0;
    state.n_recv = 0;
    state.dhr_pub = Some(*their_dh_pub);

    // Generate new DH keypair
    let (new_seed, new_pub) = dh::generate_ratchet_keypair()?;
    let our_key = dh::reusable_from_seed(&new_seed)?;

    // DH → receiving chain
    let dh_out_recv = dh::ratchet_agree(&our_key, their_dh_pub)?;
    let (new_rk, new_ckr, new_nhkr) = kdf::kdf_rk_he(&state.rk, &dh_out_recv)?;

    // DH → sending chain
    let dh_out_send = dh::ratchet_agree(&our_key, their_dh_pub)?;
    let (new_rk2, new_cks, new_nhks) = kdf::kdf_rk_he(&new_rk, &dh_out_send)?;

    state.rk = new_rk2;
    state.ckr = Some(new_ckr);
    state.cks = Some(new_cks);
    state.nhkr = new_nhkr;
    state.nhks = new_nhks;
    state.dhs_priv = Some(new_seed);
    state.dhs_pub = new_pub;

    Ok(())
}

// ── Chain advancement ──────────────────────────────────────────────

/// Skip message keys on the receiving chain up to `target_n`.
/// Stores each skipped key via the callback.
pub(crate) fn skip_receiving_keys(
    state: &mut DoubleRatchetState,
    target_n: u32,
    skipped: &dyn SkippedKeyCallback,
) -> Result<Zeroizing<[u8; 32]>, RatchetError> {
    let ckr = state
        .ckr
        .as_ref()
        .ok_or_else(|| RatchetError::SessionCorrupt("no receiving chain key".into()))?;
    let hkr = state
        .hkr
        .as_ref()
        .ok_or_else(|| RatchetError::SessionCorrupt("no receiving header key".into()))?;

    let mut ck = ckr.clone();
    while state.n_recv < target_n {
        if state.n_recv >= MAX_SKIP_PER_CHAIN {
            return Err(RatchetError::DrSkipLimit {
                max: MAX_SKIP_PER_CHAIN,
            });
        }
        let (next, skip_mk) = kdf::kdf_ck(&ck);
        skipped.store_skipped(hkr, state.n_recv, &skip_mk)?;
        ck = next;
        state.n_recv += 1;
    }

    // Derive message key for the target counter
    let (next_ck, mk) = kdf::kdf_ck(&ck);
    state.ckr = Some(next_ck);
    state.n_recv += 1;

    Ok(mk)
}

// ── Encrypt ────────────────────────────────────────────────────────

/// Encrypt a message using the HE-DR sending chain.
/// Used by the Triple Ratchet's `triple::encrypt` which may mix the
/// message key with SPQR before AEAD. Also usable standalone for EC-only.
pub fn encrypt_he(
    state: &mut DoubleRatchetState,
    plaintext: &[u8],
) -> Result<EncryptedMessage, RatchetError> {
    let cks = state.cks.as_ref().ok_or_else(|| {
        RatchetError::SessionCorrupt("no sending chain key".into())
    })?;
    let hks = state.hks.as_ref().ok_or_else(|| {
        RatchetError::SessionCorrupt("no sending header key".into())
    })?;

    let (next_ck, mk) = kdf::kdf_ck(cks);
    state.cks = Some(next_ck);

    let header = MessageHeader {
        dh_pub: state.dhs_pub,
        pn: state.pn,
        n: state.n_send,
    };
    let header_bytes = header.to_bytes();
    let hk_key = aead::build_key(hks)?;
    let mut enc_header = header_bytes.to_vec();
    aead::seal(&hk_key, state.n_send, &[], &mut enc_header)?;

    let mk_key = aead::build_key(&mk)?;
    let mut ciphertext = plaintext.to_vec();
    aead::seal(&mk_key, state.n_send, &enc_header, &mut ciphertext)?;

    state.n_send = state
        .n_send
        .checked_add(1)
        .ok_or(RatchetError::DrCounterOverflow)?;

    Ok(EncryptedMessage {
        encrypted_header: enc_header,
        ciphertext,
    })
}

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

use tracing::debug;
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
pub fn perform_dh_ratchet(
    state: &mut DoubleRatchetState,
    their_dh_pub: &[u8; 32],
    skipped: &dyn SkippedKeyCallback,
) -> Result<(), RatchetError> {
    debug!(
        n_send = state.n_send,
        n_recv = state.n_recv,
        pn = state.pn,
        "ec::perform_dh_ratchet: entering"
    );

    // Skip remaining keys on old receiving chain
    {
        let mut ck = state.ckr.clone();
        while state.n_recv < state.pn && state.n_recv < MAX_SKIP_PER_CHAIN {
            let (next, skip_mk) = kdf::kdf_ck(&ck);
            skipped.store_skipped(&state.hkr, state.n_recv, &skip_mk)?;
            ck = next;
            state.n_recv += 1;
        }
    }

    // Promote NHK → HK, reset counters
    state.hks = state.nhks.clone();
    state.hkr = state.nhkr.clone();
    state.pn = state.n_send;
    state.n_send = 0;
    state.n_recv = 0;
    state.dhr_pub = *their_dh_pub;

    // DH → receiving chain using EXISTING key (Signal DR HE §4 step 7)
    let existing_key = dh::reusable_from_seed(&state.dhs_priv)?;
    let dh_out_recv = dh::ratchet_agree(&existing_key, their_dh_pub)?;
    let (new_rk, new_ckr, new_nhkr) = kdf::kdf_rk_he(&state.rk, &dh_out_recv)?;

    // Generate new DH keypair AFTER CKr (Signal DR HE §4 step 8)
    let (new_seed, new_pub) = dh::generate_ratchet_keypair()?;
    let new_key = dh::reusable_from_seed(&new_seed)?;

    // DH → sending chain using NEW key (Signal DR HE §4 step 9)
    let dh_out_send = dh::ratchet_agree(&new_key, their_dh_pub)?;
    let (new_rk2, new_cks, new_nhks) = kdf::kdf_rk_he(&new_rk, &dh_out_send)?;

    state.rk = new_rk2;
    state.ckr = new_ckr;
    state.cks = new_cks;
    state.nhkr = new_nhkr;
    state.nhks = new_nhks;
    state.dhs_priv = new_seed;
    state.dhs_pub = new_pub;

    debug!("ec::perform_dh_ratchet: complete");

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
    let mut ck = state.ckr.clone();
    while state.n_recv < target_n {
        if state.n_recv >= MAX_SKIP_PER_CHAIN {
            return Err(RatchetError::DrSkipLimit {
                max: MAX_SKIP_PER_CHAIN,
            });
        }
        let (next, skip_mk) = kdf::kdf_ck(&ck);
        skipped.store_skipped(&state.hkr, state.n_recv, &skip_mk)?;
        ck = next;
        state.n_recv += 1;
    }

    // Derive message key for the target counter
    let (next_ck, mk) = kdf::kdf_ck(&ck);
    state.ckr = next_ck;
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
    debug!(
        n_send = state.n_send,
        plaintext_len = plaintext.len(),
        "ec::encrypt_he: entering"
    );

    let (next_ck, mk) = kdf::kdf_ck(&state.cks);
    state.cks = next_ck;

    let header = MessageHeader {
        dh_pub: state.dhs_pub,
        pn: state.pn,
        n: state.n_send,
    };
    let header_bytes = header.to_bytes();
    let hk_key = aead::build_key(&state.hks)?;
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

/// Decrypt a message encrypted by `encrypt_he`. Counterpart to `encrypt_he`.
///
/// Performs header decrypt, DH ratchet if needed, chain advance, and
/// body AEAD with the raw EC message key — no hybrid KDF layer.
///
/// Used by the Responder to verify the PQXDH-INIT proof message during
/// session establishment. Regular DM messages go through `triple::decrypt`
/// which applies `derive_message_key` (hybrid KDF) on top.
pub fn decrypt_he(
    state: &mut DoubleRatchetState,
    encrypted_header: &[u8],
    ciphertext: &[u8],
    skipped: &dyn SkippedKeyCallback,
) -> Result<Vec<u8>, RatchetError> {
    let header_tag_len = HEADER_LEN + aead::TAG_LEN;
    if encrypted_header.len() != header_tag_len {
        return Err(RatchetError::DrHeaderDecrypt);
    }

    // Try HKr at n_recv, then NHKr scanning
    let mut dh_ratchet_needed = false;
    let header = if let Some(h) = try_decrypt_header_at(&state.hkr, state.n_recv, encrypted_header) {
        h
    } else {
        let h = scan_nhkr(state, encrypted_header)
            .ok_or(RatchetError::DrHeaderDecrypt)?;
        dh_ratchet_needed = true;
        h
    };

    if dh_ratchet_needed {
        perform_dh_ratchet(state, &header.dh_pub, skipped)?;
    }

    let mk = skip_receiving_keys(state, header.n, skipped)?;

    // Raw AEAD — no hybrid KDF. Matches encrypt_he's seal.
    let mk_key = aead::build_key(&mk)?;
    let mut body = ciphertext.to_vec();
    let plaintext = aead::open(&mk_key, header.n, encrypted_header, &mut body)?;

    Ok(plaintext.to_vec())
}

/// Scan NHKr for a matching header counter (used by decrypt_he).
fn scan_nhkr(
    state: &DoubleRatchetState,
    encrypted_header: &[u8],
) -> Option<MessageHeader> {
    for counter in 0..MAX_SKIP_PER_CHAIN {
        if let Some(h) = try_decrypt_header_at(&state.nhkr, counter, encrypted_header) {
            if h.n == counter {
                return Some(h);
            }
        }
    }
    None
}

//! Triple Ratchet — EC Double Ratchet + SPQR combined via KDF_HYBRID.
//!
//! Per Signal DR spec §7.2: the per-message AEAD key is
//! `KDF_HYBRID(ec_mk, pq_mk)` when SPQR is active, or
//! `KDF_HYBRID_DRONLY(ec_mk)` before the first SPQR epoch completes.
//!
//! The encrypt/decrypt functions split the EC ratchet into two phases:
//! 1. Chain advancement → raw EC message key
//! 2. KDF_HYBRID mix → final AEAD key
//! This ensures the SPQR epoch secret is mixed into every message key
//! when active, providing post-quantum security on top of the EC ratchet.

use zeroize::Zeroizing;

use crate::crypto::{aead, kdf};
use crate::error::RatchetError;
use crate::ratchet::ec::{self, EncryptedMessage, MessageHeader, SkippedKeyCallback};
use crate::session::{DoubleRatchetState, MlKemBraidState, TripleRatchetSession};

/// Encrypt a plaintext message through the Triple Ratchet.
///
/// 1. Advance EC sending chain → raw EC message key
/// 2. Mix via KDF_HYBRID (or DRONLY if SPQR inactive) → final AEAD key
/// 3. Encrypt header under hks with counter nonce
/// 4. Encrypt body under final key with AD = encrypted header
pub fn encrypt(
    session: &mut TripleRatchetSession,
    plaintext: &[u8],
) -> Result<EncryptedMessage, RatchetError> {
    // Phase 1: advance EC sending chain
    let (ec_mk, header, next_ck) = advance_sending_chain(&session.ec)?;

    // Phase 2: mix with SPQR epoch secret
    let final_mk = derive_message_key(&ec_mk, session)?;

    // Phase 3: encrypt header
    let hks = session.ec.hks.as_ref().ok_or_else(|| {
        RatchetError::SessionCorrupt("no sending header key".into())
    })?;
    let hk_key = aead::build_key(hks)?;
    let header_bytes = header.to_bytes();
    let mut enc_header = header_bytes.to_vec();
    aead::seal(&hk_key, header.n, &[], &mut enc_header)?;

    // Phase 4: encrypt body
    let mk_key = aead::build_key(&final_mk)?;
    let mut ciphertext = plaintext.to_vec();
    aead::seal(&mk_key, header.n, &enc_header, &mut ciphertext)?;

    // Commit state
    session.ec.cks = Some(next_ck);
    session.ec.n_send = header
        .n
        .checked_add(1)
        .ok_or(RatchetError::DrCounterOverflow)?;
    session.last_active = now_secs();

    Ok(EncryptedMessage {
        encrypted_header: enc_header,
        ciphertext,
    })
}

/// Decrypt a message through the Triple Ratchet.
///
/// 1. EC header decrypt → identify chain, DH ratchet if needed
/// 2. Advance receiving chain → raw EC message key
/// 3. Mix via KDF_HYBRID → final AEAD key
/// 4. Decrypt body
pub fn decrypt(
    session: &mut TripleRatchetSession,
    encrypted_header: &[u8],
    ciphertext: &[u8],
    skipped: &dyn SkippedKeyCallback,
) -> Result<Vec<u8>, RatchetError> {
    // Header decrypt + DH ratchet + chain advance (mutates state in-place)
    let (ec_mk, header) =
        ec_decrypt_split(&mut session.ec, encrypted_header, skipped)?;

    let final_mk = derive_message_key(&ec_mk, session)?;

    // Decrypt body
    let mk_key = aead::build_key(&final_mk)?;
    let mut body = ciphertext.to_vec();
    let plaintext = aead::open(&mk_key, header.n, encrypted_header, &mut body)?;
    let result = plaintext.to_vec();

    session.last_active = now_secs();

    Ok(result)
}

/// Derive the final message key for a Triple Ratchet message.
pub fn derive_message_key(
    ec_mk: &Zeroizing<[u8; 32]>,
    session: &TripleRatchetSession,
) -> Result<Zeroizing<[u8; 32]>, RatchetError> {
    if session.spqr_active {
        let pq_mk = extract_epoch_secret(&session.spqr)?;
        kdf::kdf_hybrid(ec_mk, &pq_mk)
    } else {
        kdf::kdf_hybrid_dronly(ec_mk)
    }
}

/// Extract the shared secret from the current SPQR epoch.
fn extract_epoch_secret(
    spqr: &MlKemBraidState,
) -> Result<Zeroizing<[u8; 32]>, RatchetError> {
    match spqr {
        MlKemBraidState::Complete { epoch_ss, .. }
        | MlKemBraidState::CtSending { epoch_ss, .. } => Ok(epoch_ss.clone()),
        _ => Err(RatchetError::SessionCorrupt(
            "SPQR marked active but no epoch secret available".into(),
        )),
    }
}

/// `(message_key, header, next_chain_key)`.
type AdvanceResult = (Zeroizing<[u8; 32]>, MessageHeader, Zeroizing<[u8; 32]>);

/// Advance the EC sending chain without performing AEAD.
fn advance_sending_chain(
    state: &DoubleRatchetState,
) -> Result<AdvanceResult, RatchetError> {
    let cks = state.cks.as_ref().ok_or_else(|| {
        RatchetError::SessionCorrupt("no sending chain key".into())
    })?;
    let (next_ck, mk) = kdf::kdf_ck(cks);
    let header = MessageHeader {
        dh_pub: state.dhs_pub,
        pn: state.pn,
        n: state.n_send,
    };
    Ok((mk, header, next_ck))
}

/// EC decrypt split: header decrypt + DH ratchet + chain advance, without body AEAD.
/// The caller performs body AEAD with the hybrid-mixed key.
fn ec_decrypt_split(
    state: &mut DoubleRatchetState,
    encrypted_header: &[u8],
    skipped: &dyn SkippedKeyCallback,
) -> Result<(Zeroizing<[u8; 32]>, MessageHeader), RatchetError> {
    let header_tag_len = 40 + aead::TAG_LEN;
    if encrypted_header.len() != header_tag_len {
        return Err(RatchetError::DrHeaderDecrypt);
    }

    // Try HKr at n_recv, then NHKr scanning
    let mut dh_ratchet_needed = false;
    let header = if let Some(hkr) = &state.hkr {
        if let Some(h) = ec::try_decrypt_header_at(hkr, state.n_recv, encrypted_header) {
            h
        } else {
            scan_nhkr_header(&state.nhkr, encrypted_header)
                .inspect(|_| { dh_ratchet_needed = true; })
                .ok_or(RatchetError::DrHeaderDecrypt)?
        }
    } else {
        let h = scan_nhkr_header(&state.nhkr, encrypted_header)
            .ok_or(RatchetError::DrHeaderDecrypt)?;
        dh_ratchet_needed = true;
        h
    };

    if dh_ratchet_needed {
        ec::perform_dh_ratchet(state, &header.dh_pub, skipped)?;
    }

    let mk = ec::skip_receiving_keys(state, header.n, skipped)?;

    Ok((mk, header))
}

fn scan_nhkr_header(
    nhkr: &Zeroizing<[u8; 32]>,
    encrypted_header: &[u8],
) -> Option<MessageHeader> {
    use crate::session::skipped::MAX_SKIP_PER_CHAIN;
    for counter in 0..MAX_SKIP_PER_CHAIN {
        if let Some(h) = ec::try_decrypt_header_at(nhkr, counter, encrypted_header) {
            if h.n == counter {
                return Some(h);
            }
        }
    }
    None
}

#[allow(clippy::cast_possible_wrap)]
fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

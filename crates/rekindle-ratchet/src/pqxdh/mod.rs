//! PQXDH rev 2 — post-quantum extended Diffie-Hellman key agreement.
//!
//! Implements Signal PQXDH Revision 3 (2024-01-23) with:
//! - F3 mitigation: algorithm-byte prefixes (0x01=X25519, 0x02=ML-KEM-768)
//! - F4 mitigation: distinct domain tags for OT vs LR PQ prekeys
//! - Separate Ed25519 and X25519 identity seeds (no XEdDSA scalar reuse)
//!
//! The session key (SK) output becomes the initial root key for the
//! Double Ratchet with Header Encryption.
//!
//! EK_A (the initiator's ephemeral key) is generated internally by
//! [`initiate`] as a `PrivateKey` — not `EphemeralPrivateKey` — because
//! the PQXDH spec requires EK_A to participate in DH2, DH3, and
//! optionally DH4 (three agreements from one scalar). `EphemeralPrivateKey`
//! is consumed on first `agree_ephemeral` call and cannot be reused.
//! `PrivateKey` borrows on `agree` and supports multiple agreements.
//! The key is ephemeral in protocol semantics (used once per handshake,
//! never persisted) but reusable within the handshake scope.

pub mod bundle;
pub mod verify;

use aws_lc_rs::hkdf;
use zeroize::Zeroizing;

use crate::crypto::{dh, kem};
use crate::error::RatchetError;
use bundle::{PqxdhInitMessage, PqxdhInitResult, PqxdhResponderResult, PreKeyBundle};

const PQXDH_INFO: &[u8] = b"Rekindle_PQXDH_SOTA_v1";
const F_PREFIX: [u8; 32] = [0xFF; 32];
const ZERO_SALT: [u8; 32] = [0u8; 32];

struct Len32;
impl hkdf::KeyType for Len32 {
    fn len(&self) -> usize {
        32
    }
}

/// Derive SK from assembled IKM via HKDF-SHA-256.
fn derive_session_key(ikm: &[u8]) -> Result<Zeroizing<[u8; 32]>, RatchetError> {
    let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, &ZERO_SALT);
    let prk = salt.extract(ikm);
    let okm = prk.expand(&[PQXDH_INFO], Len32).map_err(|_| RatchetError::Kdf)?;
    let mut sk = Zeroizing::new([0u8; 32]);
    okm.fill(sk.as_mut()).map_err(|_| RatchetError::Kdf)?;
    Ok(sk)
}

/// PQXDH initiator (Alice → Bob).
///
/// Generates EK_A internally, performs DH1-DH4 + ML-KEM encaps,
/// derives SK, and returns the session key + init message for Bob.
///
/// EK_A is a `PrivateKey` (reusable within this function scope for
/// DH2/DH3/DH4), then dropped. The seed is zeroized on function exit.
pub fn initiate(
    ik_a_dh_seed: &[u8; 32],
    ik_a_ed25519_pub: &[u8; 32],
    bundle: &PreKeyBundle,
) -> Result<PqxdhInitResult, RatchetError> {
    // Generate EK_A as PrivateKey (reusable for DH2, DH3, DH4)
    let (ek_a_seed, ek_a_pub) = dh::generate_ratchet_keypair()?;
    let ek_a = dh::reusable_from_seed(&ek_a_seed)?;

    // IK_A_DH for DH1
    let ik_a_dh = dh::reusable_from_seed(ik_a_dh_seed)?;

    // DH1 = X25519(IK_A_DH, SPK_B)
    let dh1 = dh::ratchet_agree(&ik_a_dh, &bundle.spk)?;

    // DH2 = X25519(EK_A, IK_B_DH)
    let dh2 = dh::ratchet_agree(&ek_a, &bundle.ik_x25519)?;

    // DH3 = X25519(EK_A, SPK_B)
    let dh3 = dh::ratchet_agree(&ek_a, &bundle.spk)?;

    // DH4 = X25519(EK_A, OPK_B) — optional
    let dh4 = if let Some(opk) = &bundle.opk {
        Some(dh::ratchet_agree(&ek_a, opk)?)
    } else {
        None
    };

    // ML-KEM-768 encapsulation
    let (pq_ek, is_last_resort) = bundle.pq_ek();
    let pq_ek_arr: [u8; kem::EK_LEN] = pq_ek
        .try_into()
        .map_err(|_| RatchetError::KemEkInvalid)?;
    let (kem_ct, ss) = kem::encaps(&pq_ek_arr)?;

    // IKM = F || DH1 || DH2 || DH3 [|| DH4] || SS
    let mut ikm = Vec::with_capacity(32 + 32 * 5);
    ikm.extend_from_slice(&F_PREFIX);
    ikm.extend_from_slice(dh1.as_ref());
    ikm.extend_from_slice(dh2.as_ref());
    ikm.extend_from_slice(dh3.as_ref());
    if let Some(ref dh4_out) = dh4 {
        ikm.extend_from_slice(dh4_out.as_ref());
    }
    ikm.extend_from_slice(ss.as_ref());

    let sk = derive_session_key(&ikm)?;
    ikm.fill(0);

    // Select the PQ prekey ID used
    let pqpk_b_id = if is_last_resort { 0 } else { bundle.pqpk_ot_id };

    // Initialize the Double Ratchet as initiator
    let (dr_seed, dr_pub) = dh::generate_ratchet_keypair()?;
    let mut ec_state = crate::session::DoubleRatchetState::init_initiator(
        &sk,
        dr_seed,
        dr_pub,
        bundle.spk,
    )?;

    // Produce the first DR-encrypted message (proves SK possession)
    let first_msg = crate::ratchet::ec::encrypt_he(&mut ec_state, b"PQXDH-INIT")?;

    let ik_a_x25519_pub: [u8; 32] = ik_a_dh
        .compute_public_key()
        .map_err(|_| RatchetError::Ecdh)?
        .as_ref()
        .try_into()
        .map_err(|_| RatchetError::Ecdh)?;

    let init_message = PqxdhInitMessage {
        ik_a_x25519: ik_a_x25519_pub,
        ik_a_ed25519: *ik_a_ed25519_pub,
        ek_a: ek_a_pub,
        spk_b_id: bundle.spk_id,
        pqpk_b_id,
        pqpk_is_last_resort: is_last_resort,
        opk_b_id: bundle.opk_id,
        kem_ct: kem_ct.to_vec(),
        initial_ciphertext: first_msg.ciphertext,
    };

    Ok(PqxdhInitResult {
        session_key: sk,
        init_message,
        ec_state,
    })
}

/// PQXDH responder (Bob processes Alice's init message).
///
/// Bob uses his stored private key seeds to compute the same
/// DH1-DH4 + ML-KEM decaps, deriving the identical SK.
pub fn respond(
    ik_b_dh_seed: &[u8; 32],
    spk_b_seed: &[u8; 32],
    opk_b_seed: Option<&[u8; 32]>,
    pqpk_dk: &Zeroizing<[u8; kem::DK_LEN]>,
    init_msg: &PqxdhInitMessage,
) -> Result<PqxdhResponderResult, RatchetError> {
    let ik_b_dh = dh::reusable_from_seed(ik_b_dh_seed)?;
    let spk_b = dh::reusable_from_seed(spk_b_seed)?;

    // DH1 = X25519(SPK_B, IK_A_DH)
    let dh1 = dh::ratchet_agree(&spk_b, &init_msg.ik_a_x25519)?;

    // DH2 = X25519(IK_B_DH, EK_A)
    let dh2 = dh::ratchet_agree(&ik_b_dh, &init_msg.ek_a)?;

    // DH3 = X25519(SPK_B, EK_A)
    let dh3 = dh::ratchet_agree(&spk_b, &init_msg.ek_a)?;

    // DH4 = X25519(OPK_B, EK_A) — optional
    let dh4 = if let Some(opk_seed) = opk_b_seed {
        let opk = dh::reusable_from_seed(opk_seed)?;
        Some(dh::ratchet_agree(&opk, &init_msg.ek_a)?)
    } else {
        None
    };

    // SS = ML-KEM-768.Decaps(dk, ct)
    let ct: [u8; kem::CT_LEN] = init_msg
        .kem_ct
        .as_slice()
        .try_into()
        .map_err(|_| RatchetError::KemDecaps)?;
    let ss = kem::decaps(pqpk_dk, &ct)?;

    // IKM = F || DH1 || DH2 || DH3 [|| DH4] || SS
    let mut ikm = Vec::with_capacity(32 + 32 * 5);
    ikm.extend_from_slice(&F_PREFIX);
    ikm.extend_from_slice(dh1.as_ref());
    ikm.extend_from_slice(dh2.as_ref());
    ikm.extend_from_slice(dh3.as_ref());
    if let Some(ref dh4_out) = dh4 {
        ikm.extend_from_slice(dh4_out.as_ref());
    }
    ikm.extend_from_slice(ss.as_ref());

    let sk = derive_session_key(&ikm)?;
    ikm.fill(0);

    Ok(PqxdhResponderResult { session_key: sk })
}

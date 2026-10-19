//! PQXDH key agreement for the spec session (plan E2.2): both sides of
//! rev. 3 §3.3–§3.4, with the key material split into `SKec` (the Double
//! Ratchet's root) and `SKscka` (kept for the post-quantum ratchet, F2),
//! and the AD returned to the caller.
//!
//! The KDF input binds the keys explicitly:
//! `F ‖ DH1 ‖ DH2 ‖ DH3 [‖ DH4] ‖ SS ‖ EncodeEC(IK_A) ‖ EncodeEC(IK_B) ‖
//! EncodeKEM(PQPK_B)`. Every element has a fixed length, so the input
//! parses one way. Binding `EncodeKEM(PQPK_B)` answers rev. 3's note for a
//! KEM that may not bind its public key (it appends it to the AD; here the
//! KDF carries it, so the session keys themselves depend on it).

use ed25519_dalek::VerifyingKey;
use hkdf::Hkdf;
use rekindle_secrets::pq_keys::{ml_kem_ciphertext_from_bytes, MlKemPublic, MlKemSecret};
use rekindle_types::domains;
use rekindle_utils::random::RandomSource;
use sha2::Sha256;
use x25519_dalek::{PublicKey as X25519Public, SharedSecret, StaticSecret};
use zeroize::Zeroizing;

use super::encode::{encode_identity, encode_kem, ENCODED_EC_LEN};
use super::kdf::F_BYTES;
use super::keys::{ml_kem_public_from_bytes, x25519_from_bytes, x25519_from_ed};
use super::verify::{verify_pq, verify_spk};
use super::PqxdhError;
use crate::signal::prekeys::PreKeyBundle;
use crate::signal::session::PQ_LR_ID;
use crate::Identity;

/// Length of the AD: `EncodeEC(IK_A) ‖ EncodeEC(IK_B)`.
pub const AD_LEN: usize = 2 * ENCODED_EC_LEN;

/// The AD both sides bind into every message of the session (§3.3):
/// the initiator's identity first.
pub type AssociatedData = [u8; AD_LEN];

/// The PQXDH output, split.
pub struct SessionKeys {
    /// The Double Ratchet's initial root key.
    pub sk_ec: Zeroizing<[u8; 32]>,
    /// The key the post-quantum ratchet (F2) starts from.
    pub sk_scka: Zeroizing<[u8; 32]>,
}

/// Which ML-KEM prekey the initiator encapsulated to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PqPrekeyId {
    /// A one-time key, consumed by the responder once the first message
    /// decrypts.
    OneTime(u32),
    /// The last-resort key, which is never consumed.
    LastResort(u32),
}

/// The initiator's result: what the PreKeyInit carries and the keys.
pub struct InitiatorAgreement {
    /// `EK_A`'s public half: the session's base key.
    pub base_key: [u8; 32],
    /// The ML-KEM ciphertext `CT`.
    pub kem_ciphertext: Vec<u8>,
    /// The one-time curve prekey used for DH4, if the bundle had one.
    pub one_time_prekey_id: Option<u32>,
    /// The ML-KEM prekey `CT` is for.
    pub pq_prekey: PqPrekeyId,
    /// `SKec` and `SKscka`.
    pub keys: SessionKeys,
    /// `EncodeEC(IK_A) ‖ EncodeEC(IK_B)`.
    pub ad: AssociatedData,
}

/// The responder's secrets for one PreKeyInit, resolved by the caller
/// from the ids the init names.
pub struct ResponderKeys<'a> {
    /// Our identity (`IK_B`).
    pub identity: &'a Identity,
    /// The signed prekey the init names (`SPK_B`).
    pub signed_prekey: &'a StaticSecret,
    /// The one-time curve prekey, exactly when the init names one.
    pub one_time_prekey: Option<&'a StaticSecret>,
    /// The ML-KEM secret matching the init's [`PqPrekeyId`].
    pub pq_prekey: &'a MlKemSecret,
}

/// The initiator side (§3.3): verify the bundle's signatures under
/// `IK_B`, draw `EK_A` from `rng`, run DH1–DH4 and the encapsulation.
pub fn initiate(
    our: &Identity,
    bundle: &PreKeyBundle,
    rng: &dyn RandomSource,
) -> Result<InitiatorAgreement, PqxdhError> {
    let their_ik = identity_key(&bundle.identity_key)?;
    let (pq_target, pq_prekey) = verified_pq_target(bundle, &their_ik)?;
    let opk = match (&bundle.one_time_prekey, bundle.one_time_prekey_id) {
        (Some(key), Some(id)) => Some((x25519_from_bytes(key)?, id)),
        (None, None) => None,
        _ => {
            return Err(PqxdhError::IncompleteBundle(
                "one-time prekey without its id",
            ))
        }
    };

    let mut ek_bytes = Zeroizing::new([0u8; 32]);
    rng.fill(&mut *ek_bytes);
    let ek = StaticSecret::from(*ek_bytes);
    let spk_b = x25519_from_bytes(&bundle.signed_prekey)?;

    let mut dhs = vec![
        our.to_x25519_secret().diffie_hellman(&spk_b),
        ek.diffie_hellman(&x25519_from_ed(&their_ik)),
        ek.diffie_hellman(&spk_b),
    ];
    if let Some((opk, _)) = &opk {
        dhs.push(ek.diffie_hellman(opk));
    }
    let encap = pq_target.encapsulate();
    let (keys, ad) = derive(
        &dhs,
        &encap.shared_secret,
        &our.public_key(),
        &their_ik,
        &pq_target,
    )?;
    Ok(InitiatorAgreement {
        base_key: X25519Public::from(&ek).to_bytes(),
        kem_ciphertext: encap.ciphertext,
        one_time_prekey_id: opk.map(|(_, id)| id),
        pq_prekey,
        keys,
        ad,
    })
}

/// The responder side (§3.4): the same DHs from our secrets, the
/// decapsulation, the same KDF.
pub fn respond(
    keys: &ResponderKeys<'_>,
    initiator: &VerifyingKey,
    base_key: &[u8; 32],
    kem_ciphertext: &[u8],
) -> Result<(SessionKeys, AssociatedData), PqxdhError> {
    let ek_a = X25519Public::from(*base_key);
    let mut dhs = vec![
        keys.signed_prekey
            .diffie_hellman(&x25519_from_ed(initiator)),
        keys.identity.to_x25519_secret().diffie_hellman(&ek_a),
        keys.signed_prekey.diffie_hellman(&ek_a),
    ];
    if let Some(opk) = keys.one_time_prekey {
        dhs.push(opk.diffie_hellman(&ek_a));
    }
    let ct =
        ml_kem_ciphertext_from_bytes(kem_ciphertext).ok_or(PqxdhError::InvalidMlKemCiphertext)?;
    let ss = keys.pq_prekey.decapsulate(&ct);
    derive(
        &dhs,
        &ss,
        initiator,
        &keys.identity.public_key(),
        &keys.pq_prekey.public(),
    )
}

fn identity_key(bytes: &[u8]) -> Result<VerifyingKey, PqxdhError> {
    let array = <[u8; 32]>::try_from(bytes)
        .map_err(|_| PqxdhError::InvalidIdentityKey(format!("{} bytes", bytes.len())))?;
    VerifyingKey::from_bytes(&array).map_err(|e| PqxdhError::InvalidIdentityKey(e.to_string()))
}

/// Verify the signed prekey and the PQ keys, and pick the encapsulation
/// target: the one-time key when the bundle has one, else the last-resort
/// key (§3.3). A one-time PQ key must arrive with its signature and id.
fn verified_pq_target(
    bundle: &PreKeyBundle,
    their_ik: &VerifyingKey,
) -> Result<(MlKemPublic, PqPrekeyId), PqxdhError> {
    verify_spk(
        their_ik,
        &bundle.signed_prekey,
        &bundle.signed_prekey_signature,
    )?;
    verify_pq(their_ik, b"LR", &bundle.pqpk_lr, &bundle.pqpk_lr_signature)?;
    match (
        &bundle.pqpk_ot,
        &bundle.pqpk_ot_signature,
        bundle.pqpk_ot_id,
    ) {
        (Some(key), Some(sig), Some(id)) => {
            verify_pq(their_ik, b"OT", key, sig)?;
            Ok((ml_kem_public_from_bytes(key)?, PqPrekeyId::OneTime(id)))
        }
        (None, None, None) => Ok((
            ml_kem_public_from_bytes(&bundle.pqpk_lr)?,
            PqPrekeyId::LastResort(PQ_LR_ID),
        )),
        _ => Err(PqxdhError::IncompleteBundle(
            "one-time PQ prekey without its signature or id",
        )),
    }
}

/// The KDF over the module's input layout; refuses a non-contributory DH
/// (an all-zero output from a low-order peer key).
fn derive(
    dhs: &[SharedSecret],
    ss: &[u8; 32],
    ik_a: &VerifyingKey,
    ik_b: &VerifyingKey,
    pq: &MlKemPublic,
) -> Result<(SessionKeys, AssociatedData), PqxdhError> {
    if dhs.iter().any(|dh| !dh.was_contributory()) {
        return Err(PqxdhError::NonContributory);
    }
    let mut ad = [0u8; AD_LEN];
    ad[..ENCODED_EC_LEN].copy_from_slice(&encode_identity(ik_a));
    ad[ENCODED_EC_LEN..].copy_from_slice(&encode_identity(ik_b));

    let kem = encode_kem(pq);
    let mut ikm = Zeroizing::new(Vec::with_capacity(
        32 * (dhs.len() + 2) + AD_LEN + kem.len(),
    ));
    ikm.extend_from_slice(&F_BYTES);
    for dh in dhs {
        ikm.extend_from_slice(dh.as_bytes());
    }
    ikm.extend_from_slice(ss);
    ikm.extend_from_slice(&ad);
    ikm.extend_from_slice(&kem);

    let mut okm = Zeroizing::new([0u8; 64]);
    Hkdf::<Sha256>::new(None, &ikm)
        .expand(domains::PQXDH_SK.as_bytes(), &mut *okm)
        .map_err(|e| PqxdhError::Hkdf(e.to_string()))?;
    let mut sk_ec = Zeroizing::new([0u8; 32]);
    let mut sk_scka = Zeroizing::new([0u8; 32]);
    sk_ec.copy_from_slice(&okm[..32]);
    sk_scka.copy_from_slice(&okm[32..]);
    Ok((SessionKeys { sk_ec, sk_scka }, ad))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::test_support::{BobKeys, TestRandom};

    fn run(bob: &BobKeys, one_time: bool) -> (InitiatorAgreement, SessionKeys, AssociatedData) {
        let alice = Identity::generate();
        let bundle = bob.bundle(one_time);
        let agreed = initiate(&alice, &bundle, &TestRandom::new(1)).unwrap();
        let (keys, ad) = respond(
            &bob.responder_keys(&agreed),
            &alice.public_key(),
            &agreed.base_key,
            &agreed.kem_ciphertext,
        )
        .unwrap();
        (agreed, keys, ad)
    }

    #[test]
    fn both_sides_derive_both_keys_and_the_ad() {
        let bob = BobKeys::new();
        for one_time in [true, false] {
            let (agreed, keys, ad) = run(&bob, one_time);
            assert_eq!(*agreed.keys.sk_ec, *keys.sk_ec);
            assert_eq!(*agreed.keys.sk_scka, *keys.sk_scka);
            assert_ne!(*keys.sk_ec, *keys.sk_scka);
            assert_eq!(agreed.ad, ad);
            assert_eq!(agreed.one_time_prekey_id.is_some(), one_time);
            assert_eq!(matches!(agreed.pq_prekey, PqPrekeyId::OneTime(_)), one_time);
        }
    }

    #[test]
    fn the_ad_names_the_initiator_first() {
        let bob = BobKeys::new();
        let (_, _, ad) = run(&bob, false);
        assert_eq!(
            &ad[ENCODED_EC_LEN..],
            &encode_identity(&bob.identity.public_key())
        );
    }

    #[test]
    fn a_wrong_kem_secret_derives_other_keys() {
        let bob = BobKeys::new();
        let alice = Identity::generate();
        let agreed = initiate(&alice, &bob.bundle(true), &TestRandom::new(2)).unwrap();
        let mut keys = bob.responder_keys(&agreed);
        keys.pq_prekey = &bob.pq_last_resort;
        let (derived, _) = respond(
            &keys,
            &alice.public_key(),
            &agreed.base_key,
            &agreed.kem_ciphertext,
        )
        .unwrap();
        assert_ne!(*derived.sk_ec, *agreed.keys.sk_ec);
    }

    #[test]
    fn an_unsigned_one_time_pq_key_is_refused() {
        let bob = BobKeys::new();
        let mut bundle = bob.bundle(true);
        bundle.pqpk_ot_signature = None;
        let result = initiate(&Identity::generate(), &bundle, &TestRandom::new(3));
        assert!(matches!(result, Err(PqxdhError::IncompleteBundle(_))));
    }

    #[test]
    fn a_low_order_base_key_is_refused() {
        let bob = BobKeys::new();
        let alice = Identity::generate();
        let agreed = initiate(&alice, &bob.bundle(false), &TestRandom::new(4)).unwrap();
        let result = respond(
            &bob.responder_keys(&agreed),
            &alice.public_key(),
            &[0u8; 32],
            &agreed.kem_ciphertext,
        );
        assert!(matches!(result, Err(PqxdhError::NonContributory)));
    }
}

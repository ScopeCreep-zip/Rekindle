//! PQXDH initiator + responder handshake functions.
//!
//! Phase 3a built these in isolation; Phase 3b wired them into
//! `SignalSessionManager::{establish_session, respond_to_session}` and
//! their daemon-track mirrors in rekindle-transport. The end-to-end
//! tests below (`initiator_and_responder_derive_identical_root_key` and
//! `last_resort_path_derives_identical_root_key`) prove handshake parity
//! using the production `to_scalar_bytes` derivation, with no session
//! machinery in the path.

use ed25519_dalek::VerifyingKey;
use rand::rngs::OsRng;
use rekindle_secrets::pq_keys::{ml_kem_ciphertext_from_bytes, MlKemSecret};
use x25519_dalek::{PublicKey as X25519Public, StaticSecret};
use zeroize::Zeroizing;

use super::bundle::PqPreKeyBundle;
use super::kdf::derive_root_key;
use super::keys::{ml_kem_public_from_bytes, x25519_from_bytes, x25519_from_ed};
use super::verify::{verify_pq, verify_spk};
use super::PqxdhError;

/// Output of the initiator-side PQXDH handshake. The initiator transmits
/// these fields to the responder so the responder can reproduce the
/// `root_key` symmetrically.
pub struct InitiatorHandshake {
    /// Ephemeral X25519 public key (`EK_A`) the initiator used for
    /// DH2/DH3/DH4.
    pub ek_public: [u8; 32],
    /// ML-KEM ciphertext (1088 bytes) encapsulated to the responder's
    /// chosen PQ key. Responder decapsulates with the matching secret.
    pub ml_kem_ct: Vec<u8>,
    /// ID of the one-time PQ prekey the initiator consumed (so the
    /// responder knows which `MlKemSecret` to use for decapsulation).
    /// `None` means the last-resort key was used.
    pub used_ot_pqpk_id: Option<u32>,
    /// ID of the one-time X25519 prekey the initiator consumed (`None`
    /// means no OPK was present in the bundle so DH4 was skipped).
    pub used_ot_opk_id: Option<u32>,
    /// Derived 32-byte session root key. Phase 3b feeds this into the
    /// Double Ratchet.
    pub root_key: Zeroizing<[u8; 32]>,
}

/// Inputs the responder needs to reproduce the initiator's `root_key`.
pub struct ResponderInput<'a> {
    /// Responder's own X25519 identity secret (Curve25519 form of IK_B).
    pub our_ik_x25519_secret: &'a StaticSecret,
    /// Responder's own X25519 signed prekey secret (SPK_B private half).
    pub our_spk_secret: &'a StaticSecret,
    /// Responder's own X25519 one-time prekey secret. Must be `Some`
    /// iff the initiator used `OPK_B` (signaled by `used_ot_opk_id`).
    pub our_opk_secret: Option<&'a StaticSecret>,
    /// Responder's ML-KEM secret matching whichever PQ key the initiator
    /// encapsulated to (one-time preferred; else last-resort).
    pub our_ml_kem_secret: &'a MlKemSecret,
    /// Initiator's Ed25519 identity verifying key.
    pub initiator_ik_ed: &'a VerifyingKey,
    /// Initiator's ephemeral X25519 public key (received over the wire).
    pub initiator_ek_public: &'a [u8],
    /// Initiator's ML-KEM ciphertext (1088 bytes).
    pub ml_kem_ciphertext: &'a [u8],
}

/// Initiator-side PQXDH. Verifies the responder's signed prekey + PQ
/// prekey, computes DH1..DH4, encapsulates a fresh ML-KEM shared secret,
/// returns the wire material + the derived `root_key`.
pub fn pqxdh_initiator(
    our_ik_x25519_secret: &StaticSecret,
    bundle: &PqPreKeyBundle,
    their_ik_ed: &VerifyingKey,
) -> Result<InitiatorHandshake, PqxdhError> {
    verify_spk(
        their_ik_ed,
        &bundle.signed_prekey,
        &bundle.signed_prekey_signature,
    )?;
    verify_pq(
        their_ik_ed,
        b"LR",
        &bundle.pqpk_lr,
        &bundle.pqpk_lr_signature,
    )?;
    if let (Some(ot), Some(sig)) = (&bundle.pqpk_ot, &bundle.pqpk_ot_signature) {
        verify_pq(their_ik_ed, b"OT", ot, sig)?;
    }

    let ek = StaticSecret::random_from_rng(OsRng);
    let ek_pub = X25519Public::from(&ek);

    let spk_b = x25519_from_bytes(&bundle.signed_prekey)?;
    let ik_b_x = x25519_from_ed(their_ik_ed);

    let dh1 = our_ik_x25519_secret.diffie_hellman(&spk_b);
    let dh2 = ek.diffie_hellman(&ik_b_x);
    let dh3 = ek.diffie_hellman(&spk_b);
    let dh4 = if let Some(opk_bytes) = &bundle.one_time_prekey {
        let opk = x25519_from_bytes(opk_bytes)?;
        Some(ek.diffie_hellman(&opk))
    } else {
        None
    };

    let (target, used_ot_pqpk_id) = if let Some(ot) = &bundle.pqpk_ot {
        (ml_kem_public_from_bytes(ot)?, bundle.pqpk_ot_id)
    } else {
        (ml_kem_public_from_bytes(&bundle.pqpk_lr)?, None)
    };
    let encap = target.encapsulate();

    let root_key = derive_root_key(&dh1, &dh2, &dh3, dh4.as_ref(), &encap.shared_secret)?;

    Ok(InitiatorHandshake {
        ek_public: ek_pub.to_bytes(),
        ml_kem_ct: encap.ciphertext,
        used_ot_pqpk_id,
        used_ot_opk_id: bundle.one_time_prekey_id,
        root_key,
    })
}

/// Responder-side PQXDH. Decapsulates the ML-KEM ciphertext, recomputes
/// DH1..DH4 (the reverse-orientation versions of the initiator's
/// computations), derives the same `root_key`.
pub fn pqxdh_responder(input: &ResponderInput<'_>) -> Result<Zeroizing<[u8; 32]>, PqxdhError> {
    let ik_a_x = x25519_from_ed(input.initiator_ik_ed);
    let ek_a = x25519_from_bytes(input.initiator_ek_public)?;

    // Initiator computed: DH1 = DH(IK_A_sec, SPK_B_pub)
    // Responder reproduces: DH1 = DH(SPK_B_sec, IK_A_pub)
    let dh1 = input.our_spk_secret.diffie_hellman(&ik_a_x);
    let dh2 = input.our_ik_x25519_secret.diffie_hellman(&ek_a);
    let dh3 = input.our_spk_secret.diffie_hellman(&ek_a);
    let dh4 = input.our_opk_secret.map(|s| s.diffie_hellman(&ek_a));

    let ct = ml_kem_ciphertext_from_bytes(input.ml_kem_ciphertext)
        .ok_or(PqxdhError::InvalidMlKemCiphertext)?;
    let ml_kem_ss = input.our_ml_kem_secret.decapsulate(&ct);

    derive_root_key(&dh1, &dh2, &dh3, dh4.as_ref(), &ml_kem_ss)
}

#[cfg(test)]
mod tests {
    //! Component tests (signature checks, ML-KEM round-trip, KDF
    //! determinism) plus Phase 3b's end-to-end parity tests at the
    //! primitive layer (no session machinery).

    use super::*;
    use super::super::verify::{pq_signing_payload, spk_signing_payload};
    use ed25519_dalek::{Signer, SigningKey};
    use rekindle_secrets::pq_keys::MlKemSecret;
    use x25519_dalek::PublicKey as X25519Public;

    /// Build a bundle signed by the given identity, optionally including
    /// one-time prekeys. Returns the bundle + the responder's secret
    /// material (so a future Phase 3b test can reproduce the responder
    /// side; Phase 3a's tests don't need them all).
    fn make_bundle(ik_signing: &SigningKey, with_one_time: bool) -> (PqPreKeyBundle, StaticSecret) {
        let ik_verifying = ik_signing.verifying_key();
        let spk_secret = StaticSecret::random_from_rng(OsRng);
        let spk_public = X25519Public::from(&spk_secret);
        let spk_sig = ik_signing
            .sign(&spk_signing_payload(&spk_public.to_bytes()))
            .to_bytes()
            .to_vec();

        let (one_time_prekey, one_time_prekey_id) = if with_one_time {
            let s = StaticSecret::random_from_rng(OsRng);
            let p = X25519Public::from(&s);
            (Some(p.to_bytes().to_vec()), Some(42))
        } else {
            (None, None)
        };

        let (_, pqpk_lr_pub) = MlKemSecret::generate();
        let pqpk_lr_sig = ik_signing
            .sign(&pq_signing_payload(b"LR", pqpk_lr_pub.as_bytes()))
            .to_bytes()
            .to_vec();
        let (pqpk_ot, pqpk_ot_signature, pqpk_ot_id) = if with_one_time {
            let (_, ot_pub) = MlKemSecret::generate();
            let sig = ik_signing
                .sign(&pq_signing_payload(b"OT", ot_pub.as_bytes()))
                .to_bytes()
                .to_vec();
            (Some(ot_pub.as_bytes().to_vec()), Some(sig), Some(99))
        } else {
            (None, None, None)
        };

        let bundle = PqPreKeyBundle {
            identity_key: ik_verifying.to_bytes().to_vec(),
            signed_prekey: spk_public.to_bytes().to_vec(),
            signed_prekey_signature: spk_sig,
            one_time_prekey,
            one_time_prekey_id,
            registration_id: 1,
            pqpk_lr: pqpk_lr_pub.as_bytes().to_vec(),
            pqpk_lr_signature: pqpk_lr_sig,
            pqpk_ot,
            pqpk_ot_signature,
            pqpk_ot_id,
        };
        (bundle, spk_secret)
    }

    #[test]
    fn initiator_runs_with_one_time_keys() {
        let ik_signing = SigningKey::generate(&mut OsRng);
        let (bundle, _spk_secret) = make_bundle(&ik_signing, true);
        let initiator_ik_x = StaticSecret::random_from_rng(OsRng);
        let hs = pqxdh_initiator(&initiator_ik_x, &bundle, &ik_signing.verifying_key())
            .expect("initiator should run with one-time keys");
        assert_eq!(hs.ek_public.len(), 32);
        assert_eq!(hs.ml_kem_ct.len(), 1088);
        assert_eq!(hs.root_key.len(), 32);
        assert_eq!(hs.used_ot_pqpk_id, Some(99));
        assert_eq!(hs.used_ot_opk_id, Some(42));
    }

    #[test]
    fn initiator_runs_with_last_resort_only() {
        let ik_signing = SigningKey::generate(&mut OsRng);
        let (bundle, _) = make_bundle(&ik_signing, false);
        let initiator_ik_x = StaticSecret::random_from_rng(OsRng);
        let hs = pqxdh_initiator(&initiator_ik_x, &bundle, &ik_signing.verifying_key())
            .expect("initiator should run with last-resort-only bundle");
        assert_eq!(hs.used_ot_pqpk_id, None);
        assert_eq!(hs.used_ot_opk_id, None);
    }

    #[test]
    fn initiator_rejects_bad_spk_signature() {
        let ik_signing = SigningKey::generate(&mut OsRng);
        let (mut bundle, _) = make_bundle(&ik_signing, false);
        bundle.signed_prekey_signature[0] ^= 0xFF;
        let initiator_ik_x = StaticSecret::random_from_rng(OsRng);
        let result = pqxdh_initiator(&initiator_ik_x, &bundle, &ik_signing.verifying_key());
        assert!(matches!(result, Err(PqxdhError::SignatureVerify(_))));
    }

    #[test]
    fn initiator_rejects_bad_pq_lr_signature() {
        let ik_signing = SigningKey::generate(&mut OsRng);
        let (mut bundle, _) = make_bundle(&ik_signing, false);
        bundle.pqpk_lr_signature[0] ^= 0xFF;
        let initiator_ik_x = StaticSecret::random_from_rng(OsRng);
        let result = pqxdh_initiator(&initiator_ik_x, &bundle, &ik_signing.verifying_key());
        assert!(matches!(result, Err(PqxdhError::SignatureVerify(_))));
    }

    #[test]
    fn initiator_rejects_bad_pq_ot_signature() {
        let ik_signing = SigningKey::generate(&mut OsRng);
        let (mut bundle, _) = make_bundle(&ik_signing, true);
        bundle.pqpk_ot_signature.as_mut().unwrap()[0] ^= 0xFF;
        let initiator_ik_x = StaticSecret::random_from_rng(OsRng);
        let result = pqxdh_initiator(&initiator_ik_x, &bundle, &ik_signing.verifying_key());
        assert!(matches!(result, Err(PqxdhError::SignatureVerify(_))));
    }

    #[test]
    fn initiator_rejects_wrong_identity_for_signature() {
        // Bundle signed by IK_A, but initiator verifies against IK_B's key.
        let ik_a = SigningKey::generate(&mut OsRng);
        let ik_b_verifying = SigningKey::generate(&mut OsRng).verifying_key();
        let (bundle, _) = make_bundle(&ik_a, false);
        let initiator_ik_x = StaticSecret::random_from_rng(OsRng);
        let result = pqxdh_initiator(&initiator_ik_x, &bundle, &ik_b_verifying);
        assert!(matches!(result, Err(PqxdhError::SignatureVerify(_))));
    }

    #[test]
    fn distinct_handshakes_produce_distinct_root_keys() {
        // Two handshakes against the same bundle must produce different
        // root_keys (because ephemeral X25519 EK + fresh ML-KEM randomness
        // differ each run). This is the freshness property.
        let ik_signing = SigningKey::generate(&mut OsRng);
        let (bundle, _) = make_bundle(&ik_signing, true);
        let initiator_ik_x = StaticSecret::random_from_rng(OsRng);
        let hs1 = pqxdh_initiator(&initiator_ik_x, &bundle, &ik_signing.verifying_key()).unwrap();
        let hs2 = pqxdh_initiator(&initiator_ik_x, &bundle, &ik_signing.verifying_key()).unwrap();
        assert_ne!(*hs1.root_key, *hs2.root_key, "ephemeral material must differ per handshake");
        assert_ne!(hs1.ek_public, hs2.ek_public);
        assert_ne!(hs1.ml_kem_ct, hs2.ml_kem_ct);
    }

    #[test]
    fn responder_decapsulates_to_known_ml_kem_secret() {
        // Responder integration with ML-KEM works: given a ciphertext
        // encapsulated to a known public, the responder's decapsulate
        // reproduces the same shared secret.
        let (sk, pk) = MlKemSecret::generate();
        let out = pk.encapsulate();
        let ct = ml_kem_ciphertext_from_bytes(&out.ciphertext).unwrap();
        let recovered = sk.decapsulate(&ct);
        assert_eq!(*out.shared_secret, *recovered);
    }

    /// Phase 3b canonical handshake parity: initiator and responder
    /// derive byte-identical `root_key` for the same shared keypair
    /// material. Uses the production Ed25519→X25519 derivation
    /// (`SigningKey::to_scalar_bytes`) that `SessionManager` invokes.
    #[test]
    fn initiator_and_responder_derive_identical_root_key() {
        use ed25519_dalek::SigningKey;
        use crate::signal::pqxdh::verify::{pq_signing_payload, spk_signing_payload};

        // ── Responder (Bob) — generates IK_B, SPK_B, OPK_B, PQ_LR, PQ_OT.
        let bob_ik_ed = SigningKey::generate(&mut OsRng);
        let bob_ik_ed_pub = bob_ik_ed.verifying_key();
        let bob_ik_x = StaticSecret::from(bob_ik_ed.to_scalar_bytes());

        let bob_spk = StaticSecret::random_from_rng(OsRng);
        let bob_spk_pub = X25519Public::from(&bob_spk);
        let spk_sig = bob_ik_ed.sign(&spk_signing_payload(bob_spk_pub.as_bytes())).to_bytes().to_vec();

        let bob_opk = StaticSecret::random_from_rng(OsRng);
        let bob_opk_pub = X25519Public::from(&bob_opk);

        let (bob_pq_lr_sec, bob_pq_lr_pub) = MlKemSecret::generate();
        let pq_lr_sig = bob_ik_ed
            .sign(&pq_signing_payload(b"LR", bob_pq_lr_pub.as_bytes()))
            .to_bytes()
            .to_vec();

        let (bob_pq_ot_sec, bob_pq_ot_pub) = MlKemSecret::generate();
        let pq_ot_sig = bob_ik_ed
            .sign(&pq_signing_payload(b"OT", bob_pq_ot_pub.as_bytes()))
            .to_bytes()
            .to_vec();

        let bundle = PqPreKeyBundle {
            identity_key: bob_ik_ed_pub.to_bytes().to_vec(),
            signed_prekey: bob_spk_pub.as_bytes().to_vec(),
            signed_prekey_signature: spk_sig,
            one_time_prekey: Some(bob_opk_pub.as_bytes().to_vec()),
            one_time_prekey_id: Some(42),
            registration_id: 1,
            pqpk_lr: bob_pq_lr_pub.as_bytes().to_vec(),
            pqpk_lr_signature: pq_lr_sig,
            pqpk_ot: Some(bob_pq_ot_pub.as_bytes().to_vec()),
            pqpk_ot_signature: Some(pq_ot_sig),
            pqpk_ot_id: Some(7),
        };

        // ── Initiator (Alice) — generates IK_A, runs initiator handshake.
        // Third arg to pqxdh_initiator is the *responder's* Ed25519 verifying
        // key (used to verify the bundle's SPK + PQ signatures).
        let alice_ik_ed = SigningKey::generate(&mut OsRng);
        let alice_ik_x = StaticSecret::from(alice_ik_ed.to_scalar_bytes());

        let hs = pqxdh_initiator(&alice_ik_x, &bundle, &bob_ik_ed_pub).unwrap();
        assert_eq!(hs.used_ot_pqpk_id, Some(7));
        assert_eq!(hs.used_ot_opk_id, Some(42));

        // ── Responder reproduces root_key from wire material + own secrets.
        let recovered = pqxdh_responder(&ResponderInput {
            our_ik_x25519_secret: &bob_ik_x,
            our_spk_secret: &bob_spk,
            our_opk_secret: Some(&bob_opk),
            our_ml_kem_secret: &bob_pq_ot_sec,
            initiator_ik_ed: &alice_ik_ed.verifying_key(),
            initiator_ek_public: &hs.ek_public,
            ml_kem_ciphertext: &hs.ml_kem_ct,
        })
        .unwrap();

        assert_eq!(*hs.root_key, *recovered, "initiator and responder must agree on root_key");
        // Suppress unused warning on the LR secret (used in the last-resort path test).
        let _ = bob_pq_lr_sec;
    }

    /// Last-resort path: responder uses PQ_LR (not PQ_OT) when the
    /// initiator's bundle had no one-time PQ prekey.
    #[test]
    fn last_resort_path_derives_identical_root_key() {
        use ed25519_dalek::SigningKey;
        use crate::signal::pqxdh::verify::{pq_signing_payload, spk_signing_payload};

        let bob_ik_ed = SigningKey::generate(&mut OsRng);
        let bob_ik_x = StaticSecret::from(bob_ik_ed.to_scalar_bytes());
        let bob_spk = StaticSecret::random_from_rng(OsRng);
        let bob_spk_pub = X25519Public::from(&bob_spk);
        let spk_sig = bob_ik_ed.sign(&spk_signing_payload(bob_spk_pub.as_bytes())).to_bytes().to_vec();
        let (bob_pq_lr_sec, bob_pq_lr_pub) = MlKemSecret::generate();
        let pq_lr_sig = bob_ik_ed
            .sign(&pq_signing_payload(b"LR", bob_pq_lr_pub.as_bytes()))
            .to_bytes()
            .to_vec();

        let bundle = PqPreKeyBundle {
            identity_key: bob_ik_ed.verifying_key().to_bytes().to_vec(),
            signed_prekey: bob_spk_pub.as_bytes().to_vec(),
            signed_prekey_signature: spk_sig,
            one_time_prekey: None,
            one_time_prekey_id: None,
            registration_id: 1,
            pqpk_lr: bob_pq_lr_pub.as_bytes().to_vec(),
            pqpk_lr_signature: pq_lr_sig,
            pqpk_ot: None,
            pqpk_ot_signature: None,
            pqpk_ot_id: None,
        };

        let alice_ik_ed = SigningKey::generate(&mut OsRng);
        let alice_ik_x = StaticSecret::from(alice_ik_ed.to_scalar_bytes());
        let hs = pqxdh_initiator(&alice_ik_x, &bundle, &bob_ik_ed.verifying_key()).unwrap();
        assert!(hs.used_ot_pqpk_id.is_none(), "must consume last-resort, not OT");
        assert!(hs.used_ot_opk_id.is_none(), "no OPK in bundle");

        let recovered = pqxdh_responder(&ResponderInput {
            our_ik_x25519_secret: &bob_ik_x,
            our_spk_secret: &bob_spk,
            our_opk_secret: None,
            our_ml_kem_secret: &bob_pq_lr_sec,
            initiator_ik_ed: &alice_ik_ed.verifying_key(),
            initiator_ek_public: &hs.ek_public,
            ml_kem_ciphertext: &hs.ml_kem_ct,
        })
        .unwrap();

        assert_eq!(*hs.root_key, *recovered, "LR-only path must agree on root_key");
    }
}

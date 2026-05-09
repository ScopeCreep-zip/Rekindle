//! PreKeyBundle validation — verification checklist per PQXDH rev 2 §4.12.
//!
//! Every step must pass before any field is consumed for PQXDH.
//! Fail-closed: any verification failure aborts the handshake entirely.

use crate::crypto::sign;
use crate::error::RatchetError;
use crate::pqxdh::bundle::PreKeyBundle;

/// Validate a `PreKeyBundle` per PQXDH rev 2 §4.12.
///
/// 1. Verify SPK signature under `ik_ed25519`
/// 2. Verify PQPK_LR signature under `ik_ed25519`
/// 3. If PQPK_OT present: verify its signature under `ik_ed25519`
/// 4. Length checks: ML-KEM ek = 1184 (keys and sigs type-enforced)
/// 5. PQPK_LR and PQPK_OT signatures must differ (domain tag enforcement)
/// 6. `published_at` within 30 days
pub fn validate_bundle(bundle: &PreKeyBundle, now_secs: i64) -> Result<(), RatchetError> {
    // Signature lengths are enforced at 64 bytes by the Signature newtype's
    // Deserialize impl — no runtime length check needed here.

    // Step 1: SPK signature
    sign::verify_ec_prekey(
        &bundle.ik_ed25519,
        &bundle.spk,
        bundle.spk_signature.as_ref(),
    )?;

    // Step 2: PQPK_LR signature
    if bundle.pqpk_lr.len() != 1184 {
        return Err(RatchetError::PqxdhBundleInvalid {
            reason: format!("pqpk_lr length {} != 1184", bundle.pqpk_lr.len()),
        });
    }
    sign::verify_pq_prekey(
        &bundle.ik_ed25519,
        sign::DOMAIN_LR,
        &bundle.pqpk_lr,
        bundle.pqpk_lr_signature.as_ref(),
    )?;

    // Step 3: PQPK_OT signature (if present)
    if bundle.has_pqpk_ot() {
        if bundle.pqpk_ot.len() != 1184 {
            return Err(RatchetError::PqxdhBundleInvalid {
                reason: format!("pqpk_ot length {} != 1184", bundle.pqpk_ot.len()),
            });
        }
        sign::verify_pq_prekey(
            &bundle.ik_ed25519,
            sign::DOMAIN_OT,
            &bundle.pqpk_ot,
            bundle.pqpk_ot_signature.as_ref(),
        )?;
    }

    // Step 4: ik_ed25519, ik_x25519, spk are [u8; 32] — type-enforced.
    // Signatures are Signature (64 bytes) — type-enforced at deserialization.
    // OPK is Option<[u8; 32]> — type-enforced.

    // Step 5: PQPK_LR and PQPK_OT signatures must differ
    if bundle.has_pqpk_ot() && bundle.pqpk_lr_signature == bundle.pqpk_ot_signature {
        return Err(RatchetError::PqxdhDomainTagMismatch);
    }

    // Step 6: Freshness — published within 30 days
    let age_secs = now_secs - bundle.published_at;
    if age_secs > 30 * 86400 {
        return Err(RatchetError::PqxdhBundleInvalid {
            reason: format!("bundle expired: {age_secs}s old (max 2592000s)"),
        });
    }
    if age_secs < -300 {
        return Err(RatchetError::PqxdhBundleInvalid {
            reason: format!("bundle from the future: {age_secs}s ahead"),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::sign;

    fn make_test_bundle() -> (aws_lc_rs::signature::Ed25519KeyPair, PreKeyBundle) {
        let seed = [42u8; 32];
        let kp = sign::keypair_from_seed(&seed).unwrap();
        let ik_ed = sign::public_key_bytes(&kp);
        let ik_x = [1u8; 32];

        let spk = [2u8; 32];
        let spk_sig = sign::sign_ec_prekey(&kp, &spk);

        let pqpk_lr = vec![3u8; 1184];
        let pqpk_lr_sig = sign::sign_pq_prekey(&kp, sign::DOMAIN_LR, &pqpk_lr);

        let pqpk_ot = vec![4u8; 1184];
        let pqpk_ot_sig = sign::sign_pq_prekey(&kp, sign::DOMAIN_OT, &pqpk_ot);

        #[allow(clippy::cast_possible_wrap)]
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let bundle = PreKeyBundle {
            ik_ed25519: ik_ed,
            ik_x25519: ik_x,
            spk_id: 1,
            spk,
            spk_signature: Signature::from_bytes(spk_sig),
            opk_id: 0,
            opk: None,
            pqpk_ot_id: 1,
            pqpk_ot,
            pqpk_ot_signature: Signature::from_bytes(pqpk_ot_sig),
            pqpk_lr,
            pqpk_lr_signature: Signature::from_bytes(pqpk_lr_sig),
            published_at: now,
        };

        (kp, bundle)
    }

    #[test]
    fn valid_bundle_passes() {
        let (_, bundle) = make_test_bundle();
        let now = bundle.published_at;
        assert!(validate_bundle(&bundle, now).is_ok());
    }

    #[test]
    fn tampered_spk_fails() {
        let (_, mut bundle) = make_test_bundle();
        bundle.spk[0] ^= 0xFF;
        let now = bundle.published_at;
        assert!(matches!(
            validate_bundle(&bundle, now),
            Err(RatchetError::PqxdhSigInvalid)
        ));
    }

    #[test]
    fn expired_bundle_fails() {
        let (_, bundle) = make_test_bundle();
        let future = bundle.published_at + 31 * 86400;
        assert!(matches!(
            validate_bundle(&bundle, future),
            Err(RatchetError::PqxdhBundleInvalid { .. })
        ));
    }

    #[test]
    fn same_lr_ot_sig_fails() {
        let (_, mut bundle) = make_test_bundle();
        bundle.pqpk_ot_signature = bundle.pqpk_lr_signature.clone();
        let now = bundle.published_at;
        assert!(matches!(
            validate_bundle(&bundle, now),
            Err(RatchetError::PqxdhDomainTagMismatch)
        ));
    }
}

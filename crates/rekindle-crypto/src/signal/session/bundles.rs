//! `PreKeyBundle` lifecycle: reload an existing bundle from the prekey
//! store without regenerating keys, or mint a fresh one for DHT
//! publication.

use crate::error::CryptoError;
use crate::signal::pqxdh::verify::{pq_signing_payload, spk_signing_payload};
use crate::signal::prekeys::PreKeyBundle;
use crate::signal::store::PqKeyKind;

use ed25519_dalek::{Signer, SigningKey};
use rekindle_secrets::pq_keys::MlKemSecret;
use x25519_dalek::{PublicKey as X25519Public, StaticSecret};

use super::{SignalSessionManager, PQ_LR_ID};

impl SignalSessionManager {
    /// P1.2 — load an existing `PreKeyBundle` from the persisted prekey
    /// store WITHOUT regenerating keys.
    ///
    /// Returns `Ok(Some(bundle))` when both the signed prekey
    /// (`signed_prekey_id`) and — if requested — the one-time prekey
    /// (`one_time_prekey_id`) are already in the store. Reconstructs
    /// the public side from the stored X25519 secret and re-signs the
    /// signed-prekey public bytes with the identity key.
    ///
    /// Returns `Ok(None)` when any required prekey is missing — caller
    /// should call `generate_prekey_bundle` to mint fresh keys.
    ///
    /// **Why this exists**: a Stronghold-backed prekey store survives
    /// restart; calling `generate_prekey_bundle` unconditionally on
    /// every login overwrites prekey #1 + signed_prekey #1 in
    /// Stronghold AND publishes a fresh bundle to DHT subkey 5,
    /// breaking peers' cached PreKeyBundles and any in-flight messages
    /// encrypted to the previous bundle. This method gives callers a
    /// "use existing if present" path so steady-state logins reuse the
    /// already-published bundle.
    pub fn load_existing_prekey_bundle(
        &self,
        signed_prekey_id: u32,
        one_time_prekey_id: Option<u32>,
        pq_one_time_id: Option<u32>,
    ) -> Result<Option<PreKeyBundle>, CryptoError> {
        let Some(signed_prekey_secret_bytes) = self.prekeys.load_signed_prekey(signed_prekey_id)?
        else {
            return Ok(None);
        };

        let one_time_prekey_bytes = if let Some(otpk_id) = one_time_prekey_id {
            match self.prekeys.load_prekey(otpk_id)? {
                Some(bytes) => Some(bytes),
                None => return Ok(None), // requested OTPK missing → caller mints fresh
            }
        } else {
            None
        };

        // Phase 3b — PQ last-resort key must exist; if missing, caller
        // mints fresh.
        let Some(pq_lr_bytes) = self
            .prekeys
            .load_pq_secret(PQ_LR_ID, PqKeyKind::LastResort)?
        else {
            return Ok(None);
        };
        let pq_lr_secret = MlKemSecret::from_secret_bytes(&pq_lr_bytes)
            .ok_or_else(|| CryptoError::InvalidKey("PQ LR secret wrong length".into()))?;
        let pq_lr_public = pq_lr_secret.public();

        // Phase 3b — PQ one-time key (optional).
        let pq_ot_secret_opt = if let Some(id) = pq_one_time_id {
            match self.prekeys.load_pq_secret(id, PqKeyKind::OneTime)? {
                Some(bytes) => {
                    Some(MlKemSecret::from_secret_bytes(&bytes).ok_or_else(|| {
                        CryptoError::InvalidKey("PQ OT secret wrong length".into())
                    })?)
                }
                None => return Ok(None),
            }
        } else {
            None
        };

        let (identity_private, identity_public) = self.identity.get_identity_key_pair()?;
        let registration_id = self.identity.get_local_registration_id()?;

        // Reconstruct the X25519 public side of the stored signed prekey.
        let secret_array: [u8; 32] = <[u8; 32]>::try_from(&signed_prekey_secret_bytes[..])
            .map_err(|_| CryptoError::InvalidKey("signed prekey wrong length".into()))?;
        let signed_prekey_secret = StaticSecret::from(secret_array);
        let signed_prekey_public = X25519Public::from(&signed_prekey_secret);

        // Re-sign the public bytes with the identity key, using the
        // PQXDH domain-separated payload (0x01 || SPK).
        let signing_key =
            SigningKey::from_bytes(&<[u8; 32]>::try_from(&identity_private[..32]).map_err(
                |_| CryptoError::InvalidKey("identity key wrong length for signing".into()),
            )?);
        let signed_prekey_signature = signing_key
            .sign(&spk_signing_payload(signed_prekey_public.as_bytes()))
            .to_bytes()
            .to_vec();

        // Reconstruct one-time prekey public if present.
        let one_time_prekey = if let Some(bytes) = one_time_prekey_bytes {
            let array: [u8; 32] = <[u8; 32]>::try_from(&bytes[..])
                .map_err(|_| CryptoError::InvalidKey("one-time prekey wrong length".into()))?;
            let secret = StaticSecret::from(array);
            Some(X25519Public::from(&secret).as_bytes().to_vec())
        } else {
            None
        };

        // Re-sign PQ keys with the identity (domain-separated payloads).
        let pq_lr_signature = signing_key
            .sign(&pq_signing_payload(b"LR", pq_lr_public.as_bytes()))
            .to_bytes()
            .to_vec();
        let (pq_ot, pq_ot_signature) = match pq_ot_secret_opt {
            Some(ot_secret) => {
                let ot_public = ot_secret.public();
                let sig = signing_key
                    .sign(&pq_signing_payload(b"OT", ot_public.as_bytes()))
                    .to_bytes()
                    .to_vec();
                (Some(ot_public.as_bytes().to_vec()), Some(sig))
            }
            None => (None, None),
        };

        Ok(Some(PreKeyBundle {
            identity_key: identity_public,
            signed_prekey: signed_prekey_public.as_bytes().to_vec(),
            signed_prekey_signature,
            one_time_prekey,
            one_time_prekey_id,
            registration_id,
            pqpk_lr: pq_lr_public.as_bytes().to_vec(),
            pqpk_lr_signature: pq_lr_signature,
            pqpk_ot: pq_ot,
            pqpk_ot_signature: pq_ot_signature,
            pqpk_ot_id: pq_one_time_id,
        }))
    }

    /// Generate a `PreKeyBundle` for publication to DHT.
    ///
    /// Creates a signed prekey, optional one-time prekey, mandatory PQ
    /// last-resort prekey, and optional PQ one-time prekey. Stores all
    /// secrets in the prekey store and returns the bundle.
    pub fn generate_prekey_bundle(
        &self,
        signed_prekey_id: u32,
        one_time_prekey_id: Option<u32>,
        pq_one_time_id: Option<u32>,
    ) -> Result<PreKeyBundle, CryptoError> {
        let (identity_private, identity_public) = self.identity.get_identity_key_pair()?;
        let registration_id = self.identity.get_local_registration_id()?;

        let signing_key =
            SigningKey::from_bytes(&<[u8; 32]>::try_from(&identity_private[..32]).map_err(
                |_| CryptoError::InvalidKey("identity key wrong length for signing".into()),
            )?);

        // Generate signed prekey (X25519).
        let signed_prekey_secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let signed_prekey_public = X25519Public::from(&signed_prekey_secret);
        self.prekeys
            .store_signed_prekey(signed_prekey_id, signed_prekey_secret.as_bytes())?;
        let signed_prekey_signature = signing_key
            .sign(&spk_signing_payload(signed_prekey_public.as_bytes()))
            .to_bytes()
            .to_vec();

        // Optionally generate a one-time X25519 prekey.
        let one_time_prekey = if let Some(otpk_id) = one_time_prekey_id {
            let otpk_secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
            let otpk_public = X25519Public::from(&otpk_secret);
            self.prekeys.store_prekey(otpk_id, otpk_secret.as_bytes())?;
            Some(otpk_public.as_bytes().to_vec())
        } else {
            None
        };

        // Generate ML-KEM-768 last-resort key (singleton, PQ_LR_ID = 0).
        let (pq_lr_secret, pq_lr_public) = MlKemSecret::generate();
        self.prekeys.store_pq_secret(
            PQ_LR_ID,
            PqKeyKind::LastResort,
            pq_lr_secret.as_secret_bytes(),
        )?;
        let pq_lr_signature = signing_key
            .sign(&pq_signing_payload(b"LR", pq_lr_public.as_bytes()))
            .to_bytes()
            .to_vec();

        // Optionally generate a one-time ML-KEM-768 prekey.
        let (pq_ot, pq_ot_signature) = if let Some(id) = pq_one_time_id {
            let (ot_secret, ot_public) = MlKemSecret::generate();
            self.prekeys
                .store_pq_secret(id, PqKeyKind::OneTime, ot_secret.as_secret_bytes())?;
            let sig = signing_key
                .sign(&pq_signing_payload(b"OT", ot_public.as_bytes()))
                .to_bytes()
                .to_vec();
            (Some(ot_public.as_bytes().to_vec()), Some(sig))
        } else {
            (None, None)
        };

        Ok(PreKeyBundle {
            identity_key: identity_public,
            signed_prekey: signed_prekey_public.as_bytes().to_vec(),
            signed_prekey_signature,
            one_time_prekey,
            one_time_prekey_id,
            registration_id,
            pqpk_lr: pq_lr_public.as_bytes().to_vec(),
            pqpk_lr_signature: pq_lr_signature,
            pqpk_ot: pq_ot,
            pqpk_ot_signature: pq_ot_signature,
            pqpk_ot_id: pq_one_time_id,
        })
    }
}

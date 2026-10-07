//! `PreKeyBundle` construction (PQXDH §3.2–§3.3).
//!
//! The signed prekey and the PQ last-resort key are long-lived: minted once
//! and reused, so every bundle a peer caches stays valid. One-time keys are
//! handed out once: each per-peer bundle carries a freshly minted X25519
//! one-time prekey and ML-KEM-768 one-time key, which the responder deletes
//! when the handshake consumes them. In Rekindle the "server" of §3.3 is
//! the sender of the bundle.

use crate::error::CryptoError;
use crate::signal::pqxdh::verify::{pq_signing_payload, spk_signing_payload};
use crate::signal::prekeys::PreKeyBundle;
use crate::signal::store::PqKeyKind;

use ed25519_dalek::{Signer, SigningKey};
use rand::RngCore;
use rekindle_secrets::pq_keys::MlKemSecret;
use x25519_dalek::{PublicKey as X25519Public, StaticSecret};

use super::{SignalSessionManager, MAX_UNCLAIMED_ONE_TIME, PQ_LR_ID, SPK_ID};

impl SignalSessionManager {
    /// The long-lived bundle: signed prekey `SPK_ID` and PQ last-resort key
    /// `PQ_LR_ID`, each minted only when absent from the store and never
    /// overwritten. Carries no one-time keys (PQXDH §3.3: "the bundle will
    /// not contain a one-time curve prekey element"). Use it for anything
    /// published or shared with more than one peer.
    pub fn current_bundle(&self) -> Result<PreKeyBundle, CryptoError> {
        let signing_key = self.signing_key()?;
        let (_, identity_public) = self.identity.get_identity_key_pair()?;
        let registration_id = self.identity.get_local_registration_id()?;

        let spk_public = X25519Public::from(&self.signed_prekey_secret()?);
        let pq_lr_public = self.pq_last_resort_secret()?.public();

        Ok(PreKeyBundle {
            identity_key: identity_public,
            signed_prekey: spk_public.as_bytes().to_vec(),
            signed_prekey_signature: signing_key
                .sign(&spk_signing_payload(spk_public.as_bytes()))
                .to_bytes()
                .to_vec(),
            one_time_prekey: None,
            one_time_prekey_id: None,
            registration_id,
            pqpk_lr: pq_lr_public.as_bytes().to_vec(),
            pqpk_lr_signature: signing_key
                .sign(&pq_signing_payload(b"LR", pq_lr_public.as_bytes()))
                .to_bytes()
                .to_vec(),
            pqpk_ot: None,
            pqpk_ot_signature: None,
            pqpk_ot_id: None,
        })
    }

    /// [`Self::current_bundle`] plus a fresh X25519 one-time prekey and a
    /// fresh ML-KEM-768 one-time key, each under a random non-zero id not
    /// already in the store (PQXDH §4.13) and persisted before return.
    /// Beyond [`MAX_UNCLAIMED_ONE_TIME`] unclaimed keys of a kind, the
    /// oldest are deleted. Use it for a bundle sent to exactly one peer.
    pub fn handout_bundle(&self) -> Result<PreKeyBundle, CryptoError> {
        let mut bundle = self.current_bundle()?;
        let signing_key = self.signing_key()?;

        let otpk_id = fresh_id(&self.prekeys.list_prekey_ids()?);
        let otpk_secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
        self.prekeys.store_prekey(otpk_id, otpk_secret.as_bytes())?;
        self.evict_oldest_prekeys()?;

        let pq_ot_id = fresh_id(&self.prekeys.list_pq_one_time_ids()?);
        let (pq_ot_secret, pq_ot_public) = MlKemSecret::generate();
        self.prekeys.store_pq_secret(
            pq_ot_id,
            PqKeyKind::OneTime,
            pq_ot_secret.as_secret_bytes(),
        )?;
        self.evict_oldest_pq_one_time()?;

        bundle.one_time_prekey = Some(X25519Public::from(&otpk_secret).as_bytes().to_vec());
        bundle.one_time_prekey_id = Some(otpk_id);
        bundle.pqpk_ot_signature = Some(
            signing_key
                .sign(&pq_signing_payload(b"OT", pq_ot_public.as_bytes()))
                .to_bytes()
                .to_vec(),
        );
        bundle.pqpk_ot = Some(pq_ot_public.as_bytes().to_vec());
        bundle.pqpk_ot_id = Some(pq_ot_id);
        Ok(bundle)
    }

    fn signing_key(&self) -> Result<SigningKey, CryptoError> {
        let (identity_private, _) = self.identity.get_identity_key_pair()?;
        let bytes =
            <[u8; 32]>::try_from(identity_private.get(..32).unwrap_or_default()).map_err(|_| {
                CryptoError::invalid_key("identity key wrong length for signing".into())
            })?;
        Ok(SigningKey::from_bytes(&bytes))
    }

    /// The stored signed prekey, minted and persisted if absent.
    fn signed_prekey_secret(&self) -> Result<StaticSecret, CryptoError> {
        if let Some(bytes) = self.prekeys.load_signed_prekey(SPK_ID)? {
            let array = <[u8; 32]>::try_from(bytes.as_slice())
                .map_err(|_| CryptoError::invalid_key("signed prekey wrong length".into()))?;
            return Ok(StaticSecret::from(array));
        }
        let secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
        self.prekeys
            .store_signed_prekey(SPK_ID, secret.as_bytes())?;
        Ok(secret)
    }

    /// The stored PQ last-resort key, minted and persisted if absent.
    fn pq_last_resort_secret(&self) -> Result<MlKemSecret, CryptoError> {
        if let Some(bytes) = self
            .prekeys
            .load_pq_secret(PQ_LR_ID, PqKeyKind::LastResort)?
        {
            return MlKemSecret::from_secret_bytes(&bytes)
                .ok_or_else(|| CryptoError::invalid_key("PQ LR secret wrong length".into()));
        }
        let (secret, _) = MlKemSecret::generate();
        self.prekeys
            .store_pq_secret(PQ_LR_ID, PqKeyKind::LastResort, secret.as_secret_bytes())?;
        Ok(secret)
    }

    fn evict_oldest_prekeys(&self) -> Result<(), CryptoError> {
        let ids = self.prekeys.list_prekey_ids()?;
        for id in &ids[..ids.len().saturating_sub(MAX_UNCLAIMED_ONE_TIME)] {
            self.prekeys.remove_prekey(*id)?;
        }
        Ok(())
    }

    fn evict_oldest_pq_one_time(&self) -> Result<(), CryptoError> {
        let ids = self.prekeys.list_pq_one_time_ids()?;
        for id in &ids[..ids.len().saturating_sub(MAX_UNCLAIMED_ONE_TIME)] {
            self.prekeys.remove_pq_secret(*id, PqKeyKind::OneTime)?;
        }
        Ok(())
    }
}

/// A random non-zero id not in `taken`.
fn fresh_id(taken: &[u32]) -> u32 {
    loop {
        let id = rand::rngs::OsRng.next_u32();
        if id != 0 && !taken.contains(&id) {
            return id;
        }
    }
}

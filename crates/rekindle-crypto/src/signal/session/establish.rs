//! PQXDH session establishment: the initiator side (`establish_session`)
//! and the responder side (`respond_to_session`).

use crate::error::CryptoError;
use crate::signal::pqxdh;
use crate::signal::prekeys::PreKeyBundle;
use crate::signal::store::PqKeyKind;

use ed25519_dalek::{SigningKey, VerifyingKey};
use rekindle_secrets::pq_keys::MlKemSecret;
use x25519_dalek::StaticSecret;

use crate::signal::ratchet::{self, RatchetState};

use super::{SessionInitInfo, SignalSessionManager, PQ_LR_ID};

impl SignalSessionManager {
    /// Establish a session with a peer using their `PreKeyBundle` (X3DH).
    ///
    /// This is the initiator side — called when we want to start a conversation
    /// with someone whose `PreKeyBundle` we fetched from DHT.
    pub fn establish_session(
        &self,
        peer_address: &str,
        bundle: &PreKeyBundle,
    ) -> Result<SessionInitInfo, CryptoError> {
        // PQXDH initiator (Phase 3b): replaces the classical X3DH body.
        // The Ed25519 identity is converted to X25519 form via the
        // standard scalar derivation; PQ keys are verified, ML-KEM
        // ciphertext is encapsulated, root_key is derived via the
        // PQXDH KDF (F || DH1 || DH2 || DH3 [|| DH4] || SS).

        // 1. Our X25519 identity secret (Ed25519 → X25519 via scalar).
        let (identity_private, _identity_public) = self.identity.get_identity_key_pair()?;
        let identity_signing = SigningKey::from_bytes(
            &<[u8; 32]>::try_from(&identity_private[..32])
                .map_err(|_| CryptoError::invalid_key("identity key wrong length".into()))?,
        );
        let our_ik_x25519 = StaticSecret::from(identity_signing.to_scalar_bytes());

        // 2. Their Ed25519 identity (verifying key) — used for signature
        //    verification on SPK and PQ keys.
        let their_ik_ed = VerifyingKey::from_bytes(
            &<[u8; 32]>::try_from(bundle.identity_key.as_slice())
                .map_err(|_| CryptoError::invalid_key("their identity key wrong length".into()))?,
        )
        .map_err(|e| CryptoError::invalid_key(format!("their identity key not on curve: {e}")))?;

        // 3. Run the PQXDH initiator handshake against the bundle.
        let hs = pqxdh::pqxdh_initiator(&our_ik_x25519, bundle, &their_ik_ed)
            .map_err(|e| CryptoError::SessionError(format!("PQXDH initiator: {e}")))?;

        // 4. Expand the PQXDH root_key and seed the shared Double
        //    Ratchet core as initiator. `ek_secret` (not the public!)
        //    seeds our ratchet secret so the responder's first reply
        //    can complete the mirrored DH step.
        let okm = ratchet::expand_pqxdh_root(&hs.root_key)?;
        let ratchet = RatchetState::initiator(&okm, *hs.ek_secret, bundle.signed_prekey.clone());
        self.sessions
            .store_session(peer_address, &ratchet.serialize())?;

        // Trust their identity on first use (TOFU)
        self.identity
            .save_identity(peer_address, &bundle.identity_key)?;

        Ok(SessionInitInfo {
            ephemeral_public_key: hs.ek_public.to_vec(),
            // SPK id 1 matches generate_prekey_bundle(1, ...) convention.
            signed_prekey_id: 1,
            one_time_prekey_id: hs.used_ot_opk_id,
            ml_kem_ciphertext: hs.ml_kem_ct,
            used_ot_pqpk_id: hs.used_ot_pqpk_id,
        })
    }

    /// Respond to a session initiated by a peer (responder-side X3DH).
    ///
    /// Called when we receive a friend request or initial message containing
    /// the initiator's identity key and ephemeral public key.
    pub fn respond_to_session(
        &self,
        peer_address: &str,
        their_identity_key: &[u8],
        their_ephemeral_key: &[u8],
        signed_prekey_id: u32,
        one_time_prekey_id: Option<u32>,
        ml_kem_ciphertext: &[u8],
        used_ot_pqpk_id: Option<u32>,
    ) -> Result<(), CryptoError> {
        // PQXDH responder (Phase 3b): mirror of `establish_session`.
        // Loads our X25519 identity, signed prekey, optional OPK, and
        // the ML-KEM secret matching whichever PQ key the initiator
        // encapsulated to. Reconstructs the same root_key.

        // 1. Our X25519 identity (Ed25519 scalar form).
        let (identity_private, _identity_public) = self.identity.get_identity_key_pair()?;
        let identity_signing = SigningKey::from_bytes(
            &<[u8; 32]>::try_from(&identity_private[..32])
                .map_err(|_| CryptoError::invalid_key("identity key wrong length".into()))?,
        );
        let our_ik_x25519 = StaticSecret::from(identity_signing.to_scalar_bytes());

        // 2. Our signed prekey secret.
        let spk_data = self
            .prekeys
            .load_signed_prekey(signed_prekey_id)?
            .ok_or_else(|| CryptoError::invalid_key("signed prekey not found".into()))?;
        let our_spk_secret = StaticSecret::from(
            <[u8; 32]>::try_from(spk_data.as_slice())
                .map_err(|_| CryptoError::invalid_key("signed prekey wrong length".into()))?,
        );

        // 3. Our optional one-time prekey secret.
        let our_opk_secret = if let Some(otpk_id) = one_time_prekey_id {
            let otpk_data = self
                .prekeys
                .load_prekey(otpk_id)?
                .ok_or_else(|| CryptoError::invalid_key("one-time prekey not found".into()))?;
            Some(StaticSecret::from(
                <[u8; 32]>::try_from(otpk_data.as_slice())
                    .map_err(|_| CryptoError::invalid_key("one-time prekey wrong length".into()))?,
            ))
        } else {
            None
        };

        // 4. Our ML-KEM secret (one-time preferred; else last-resort).
        let (pq_kind, pq_id) = match used_ot_pqpk_id {
            Some(id) => (PqKeyKind::OneTime, id),
            None => (PqKeyKind::LastResort, PQ_LR_ID),
        };
        let pq_secret_bytes = self
            .prekeys
            .load_pq_secret(pq_id, pq_kind)?
            .ok_or_else(|| {
                CryptoError::invalid_key(format!(
                    "ML-KEM secret not found for ({pq_id}, {pq_kind:?})"
                ))
            })?;
        let our_ml_kem_secret = MlKemSecret::from_secret_bytes(&pq_secret_bytes)
            .ok_or_else(|| CryptoError::invalid_key("ML-KEM secret wrong length".into()))?;

        // 5. Initiator's Ed25519 identity (for X25519 DH partner derivation).
        let initiator_ik_ed = VerifyingKey::from_bytes(
            &<[u8; 32]>::try_from(their_identity_key)
                .map_err(|_| CryptoError::invalid_key("their identity key wrong length".into()))?,
        )
        .map_err(|e| CryptoError::invalid_key(format!("their identity key not on curve: {e}")))?;

        // 6. Run the PQXDH responder.
        let root_key_z = pqxdh::pqxdh_responder(&pqxdh::ResponderInput {
            our_ik_x25519_secret: &our_ik_x25519,
            our_spk_secret: &our_spk_secret,
            our_opk_secret: our_opk_secret.as_ref(),
            our_ml_kem_secret: &our_ml_kem_secret,
            initiator_ik_ed: &initiator_ik_ed,
            initiator_ek_public: their_ephemeral_key,
            ml_kem_ciphertext,
        })
        .map_err(|e| CryptoError::SessionError(format!("PQXDH responder: {e}")))?;

        // 7. Consume one-time keys (PQ OT + X25519 OPK).
        if pq_kind == PqKeyKind::OneTime {
            self.prekeys.remove_pq_secret(pq_id, PqKeyKind::OneTime)?;
        }
        if let Some(otpk_id) = one_time_prekey_id {
            self.prekeys.remove_prekey(otpk_id)?;
        }

        // 8. Expand the root_key and seed the shared Double Ratchet
        //    core as responder (chain assignment mirrors the
        //    initiator's). The SPK SECRET seeds our ratchet secret —
        //    the initiator's first message DHs against SPK_B.
        let okm = ratchet::expand_pqxdh_root(&root_key_z)?;
        let ratchet = RatchetState::responder(
            &okm,
            our_spk_secret.to_bytes(),
            their_ephemeral_key.to_vec(),
        );
        self.sessions
            .store_session(peer_address, &ratchet.serialize())?;

        // Trust their identity on first use (TOFU)
        self.identity
            .save_identity(peer_address, their_identity_key)?;

        Ok(())
    }
}

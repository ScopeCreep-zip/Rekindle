//! Two-phase key rotation coordinator.

use crate::v4::codec::channel::rotate::{RotateInitPayload, RotateCommitPayload};
use crate::v4::crypto::keys::{derive_rotation_keys, DerivedKeys};
use crate::v4::crypto::noise::generate_keypair;

#[derive(Debug)]
pub enum RotationError {
    RotationInProgress,
    RotationIdMismatch,
    KeygenFailed(String),
}

/// State held during an in-progress rotation.
struct PendingRotation {
    rotation_id: uuid::Uuid,
    local_chain_secret: [u8; 32],
    local_new_keypair: snow::Keypair,
}

/// Coordinates the two-phase key rotation lifecycle.
pub struct RotationCoordinator {
    current_keys: DerivedKeys,
    generation: u64,
    pending: Option<PendingRotation>,
    last_combined_secret: Option<[u8; 32]>,
    last_rotation_id: Option<uuid::Uuid>,
    new_static_pub: Option<[u8; 32]>,
}

impl RotationCoordinator {
    pub fn new(initial_keys: DerivedKeys, generation: u64) -> Self {
        Self {
            current_keys: initial_keys,
            generation,
            pending: None,
            last_combined_secret: None,
            last_rotation_id: None,
            new_static_pub: None,
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn current_keys(&self) -> &DerivedKeys {
        &self.current_keys
    }

    /// Phase 1: Initiate rotation. Generates a new keypair and random chain secret.
    pub fn initiate(&mut self, phase_2_deadline_ms: u64, transcript_anchor: [u8; 32]) -> Result<RotateInitPayload, RotationError> {
        if self.pending.is_some() {
            return Err(RotationError::RotationInProgress);
        }

        let keypair = generate_keypair()
            .map_err(|e| RotationError::KeygenFailed(format!("{e:?}")))?;

        let mut chain_secret = [0u8; 32];
        aws_lc_rs::rand::fill(&mut chain_secret)
            .expect("RNG fill failed");

        let rotation_id = uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));

        let mut new_static_pub = [0u8; 32];
        new_static_pub.copy_from_slice(&keypair.public);

        let payload = RotateInitPayload {
            rotation_id,
            initiator_generation: self.generation + 1,
            phase_2_deadline_ms,
            new_static_pub,
            rotation_chain_secret: chain_secret,
            transcript_anchor,
        };

        self.pending = Some(PendingRotation {
            rotation_id,
            local_chain_secret: chain_secret,
            local_new_keypair: keypair,
        });

        Ok(payload)
    }

    /// Responder: receive ROTATE_INIT and produce ROTATE_COMMIT.
    pub fn receive_init(&mut self, init: &RotateInitPayload, transcript_anchor: [u8; 32]) -> Result<RotateCommitPayload, RotationError> {
        let keypair = generate_keypair()
            .map_err(|e| RotationError::KeygenFailed(format!("{e:?}")))?;

        let mut chain_secret = [0u8; 32];
        aws_lc_rs::rand::fill(&mut chain_secret)
            .expect("RNG fill failed");

        let mut new_static_pub = [0u8; 32];
        new_static_pub.copy_from_slice(&keypair.public);

        // Compute combined secret and swap keys immediately for the responder
        let combined = combine_secrets(&init.rotation_chain_secret, &chain_secret);
        let old_generation = self.generation;
        self.current_keys = derive_rotation_keys(&combined);
        self.generation += 1;
        self.last_combined_secret = Some(combined);
        self.last_rotation_id = Some(init.rotation_id);
        tracing::debug!(
            old_generation,
            new_generation = self.generation,
            rotation_id = %init.rotation_id,
            envelope_d2l_fp = %hex::encode(&self.current_keys.envelope_d2l[..8]),
            envelope_l2d_fp = %hex::encode(&self.current_keys.envelope_l2d[..8]),
            "RotationCoordinator::receive_init — responder keys derived"
        );

        Ok(RotateCommitPayload {
            rotation_id: init.rotation_id,
            responder_generation: self.generation,
            new_static_pub,
            rotation_chain_secret: chain_secret,
            transcript_anchor,
        })
    }

    /// Initiator: receive ROTATE_COMMIT and complete the rotation.
    pub fn receive_commit(&mut self, commit: &RotateCommitPayload) -> Result<(), RotationError> {
        let pending = self.pending.take()
            .ok_or(RotationError::RotationIdMismatch)?;

        if commit.rotation_id != pending.rotation_id {
            // Put pending back — the rotation is still in progress
            self.pending = Some(pending);
            return Err(RotationError::RotationIdMismatch);
        }

        let mut initiator_pub = [0u8; 32];
        initiator_pub.copy_from_slice(&pending.local_new_keypair.public);

        let combined = combine_secrets(&pending.local_chain_secret, &commit.rotation_chain_secret);
        let old_generation = self.generation;
        self.current_keys = derive_rotation_keys(&combined);
        self.generation += 1;
        self.last_combined_secret = Some(combined);
        self.last_rotation_id = Some(commit.rotation_id);
        self.new_static_pub = Some(initiator_pub);
        tracing::debug!(
            old_generation,
            new_generation = self.generation,
            rotation_id = %commit.rotation_id,
            envelope_d2l_fp = %hex::encode(&self.current_keys.envelope_d2l[..8]),
            envelope_l2d_fp = %hex::encode(&self.current_keys.envelope_l2d[..8]),
            "RotationCoordinator::receive_commit — initiator keys derived"
        );

        Ok(())
    }

    /// Compute the rotation link for bridging the audit chain.
    /// Available after a completed rotation (either side).
    pub fn rotation_link(&self, pre_terminal_link: &[u8; 32]) -> Option<[u8; 32]> {
        let combined = self.last_combined_secret.as_ref()?;
        let rotation_id = self.last_rotation_id.as_ref()?;
        Some(compute_rotation_link(
            combined,
            pre_terminal_link,
            rotation_id,
            self.generation,
            self.generation,
        ))
    }

    /// The new static public key after rotation, for registry update.
    pub fn new_static_pub(&self) -> Option<&[u8; 32]> {
        self.new_static_pub.as_ref()
    }
}

/// Combine initiator and responder chain secrets into one HKDF input.
fn combine_secrets(initiator: &[u8; 32], responder: &[u8; 32]) -> [u8; 32] {
    let mut combined_input = [0u8; 64];
    combined_input[..32].copy_from_slice(initiator);
    combined_input[32..].copy_from_slice(responder);
    *blake3::hash(&combined_input).as_bytes()
}

/// Compute the rotation link that bridges pre-rotation and post-rotation
/// audit chains. The rotation link becomes the Anchor for the post-rotation chain.
pub fn compute_rotation_link(
    combined_secret: &[u8; 32],
    pre_terminal_link: &[u8; 32],
    rotation_id: &uuid::Uuid,
    initiator_generation: u64,
    responder_generation: u64,
) -> [u8; 32] {
    let mut data = Vec::with_capacity(32 + 16 + 8 + 8);
    data.extend_from_slice(pre_terminal_link);
    data.extend_from_slice(rotation_id.as_bytes());
    data.extend_from_slice(&initiator_generation.to_be_bytes());
    data.extend_from_slice(&responder_generation.to_be_bytes());

    *blake3::keyed_hash(combined_secret, &data).as_bytes()
}

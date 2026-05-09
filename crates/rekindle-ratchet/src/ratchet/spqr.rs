//! ML-KEM Braid — one-shot bridge for SPQR.
//!
//! aws-lc-rs 1.16 exposes only monolithic ML-KEM (no Encaps1/Encaps2 split).
//! This module collapses the 11-state Braid spec into a one-shot bridge:
//! receiver buffers ek_seed + ek_vector, verifies SHA3-256(ek) == hek,
//! runs one-shot encapsulate → single 1088-byte ciphertext.
//!
//! Chunk-level state machine:
//! - Sender: `KeysSampled` → encode ek_vector into shards → send header + shards
//! - Receiver: buffer header → buffer ek_vector shards → decode → encaps → encode ct shards → send
//! - Sender: buffer ct shards → decode → decaps → epoch complete
//!
//! PCS healing cost: 1 header + 45 ek shards + 43 ct shards = 89 chunks per epoch.

use aws_lc_rs::digest;
use zeroize::Zeroizing;

use crate::crypto::kem;
use crate::error::RatchetError;
use crate::ratchet::erasure::{self, ChunkKind, Shard};
use crate::session::MlKemBraidState;

/// EK_VECTOR size for ML-KEM-768 (1184 total - 32 seed = 1152).
const EK_VECTOR_LEN: usize = 1152;
/// Header size: ek_seed(32) + hek(32) = 64 bytes.
const HEADER_LEN: usize = 64;

/// Sender: generate a new ML-KEM keypair and prepare for chunked delivery.
///
/// Returns the new state (`KeysSampled`) and the header + ek_vector shards
/// ready for transport.
pub fn sender_keygen(
    epoch: u32,
) -> Result<(MlKemBraidState, Vec<Shard>), RatchetError> {
    let material = kem::keygen()?;

    let ek_seed: [u8; 32] = material.ek_bytes[..32]
        .try_into()
        .map_err(|_| RatchetError::KemKeygen)?;
    let ek_vector = material.ek_bytes[32..].to_vec();

    let hek_digest = digest::digest(&digest::SHA3_256, &material.ek_bytes);
    let mut ek_vec_hash = [0u8; 32];
    ek_vec_hash.copy_from_slice(hek_digest.as_ref());

    // Encode header: ek_seed || hek (64 bytes, single shard, no RS)
    let mut header_data = [0u8; HEADER_LEN];
    header_data[..32].copy_from_slice(&ek_seed);
    header_data[32..].copy_from_slice(&ek_vec_hash);
    let mut shards = erasure::encode(&header_data, ChunkKind::Header, epoch)?;

    // Encode ek_vector: 1152 bytes → 36 original + 9 recovery = 45 shards
    let ek_shards = erasure::encode(&ek_vector, ChunkKind::EkVector, epoch)?;
    shards.extend(ek_shards);

    let state = MlKemBraidState::KeysSampled {
        epoch,
        dk: material.dk_bytes,
        ek_seed,
        ek_vec_hash,
        ek_vector,
    };

    Ok((state, shards))
}

/// Receiver: process header shard to extract ek_seed and hek.
///
/// Returns `(ek_seed, ek_vec_hash)` or error if the header is malformed.
pub fn receiver_process_header(
    shard: &Shard,
) -> Result<([u8; 32], [u8; 32]), RatchetError> {
    if shard.kind != ChunkKind::Header || shard.data.len() < HEADER_LEN {
        return Err(RatchetError::BraidChunkReassembly { epoch: shard.epoch });
    }
    let mut ek_seed = [0u8; 32];
    let mut ek_vec_hash = [0u8; 32];
    ek_seed.copy_from_slice(&shard.data[..32]);
    ek_vec_hash.copy_from_slice(&shard.data[32..64]);
    Ok((ek_seed, ek_vec_hash))
}

/// Receiver: given enough ek_vector shards, decode + verify + encapsulate.
///
/// Returns the new state (`CtSending`) with shared secret and ciphertext
/// shards ready for transport back to the sender.
pub fn receiver_complete_ek(
    epoch: u32,
    ek_seed: &[u8; 32],
    ek_vec_hash: &[u8; 32],
    ek_shards: &[Shard],
) -> Result<(MlKemBraidState, Vec<Shard>), RatchetError> {
    // Decode ek_vector from received shards
    let n_original = EK_VECTOR_LEN.div_ceil(erasure::DATA_CHUNK_SIZE);
    let n_recovery = (n_original * erasure::RECOVERY_RATIO_NUMERATOR)
        .div_ceil(erasure::RECOVERY_RATIO_DENOMINATOR);
    let ek_vector =
        erasure::decode(ek_shards, EK_VECTOR_LEN, n_original, n_recovery)?;

    // Reconstruct full ek and verify hash
    let mut full_ek = [0u8; kem::EK_LEN];
    full_ek[..32].copy_from_slice(ek_seed);
    full_ek[32..].copy_from_slice(&ek_vector);

    let computed = digest::digest(&digest::SHA3_256, &full_ek);
    if computed.as_ref() != ek_vec_hash {
        return Err(RatchetError::BraidHekMismatch);
    }

    // One-shot encapsulate
    let (ct, ss) = kem::encaps(&full_ek)?;
    let ct_vec = ct.to_vec();

    // Encode ciphertext: 1088 bytes → 34 original + 9 recovery = 43 shards
    let ct_shards = erasure::encode(&ct_vec, ChunkKind::Ciphertext, epoch)?;

    let state = MlKemBraidState::CtSending {
        epoch,
        epoch_ss: ss,
        ct: ct_vec,
    };

    Ok((state, ct_shards))
}

/// Sender: given enough ciphertext shards, decode + decapsulate.
///
/// Returns `Complete` state with the shared secret.
pub fn sender_complete_ct(
    epoch: u32,
    dk: &Zeroizing<[u8; kem::DK_LEN]>,
    ct_shards: &[Shard],
) -> Result<MlKemBraidState, RatchetError> {
    let n_original = kem::CT_LEN.div_ceil(erasure::DATA_CHUNK_SIZE);
    let n_recovery = (n_original * erasure::RECOVERY_RATIO_NUMERATOR)
        .div_ceil(erasure::RECOVERY_RATIO_DENOMINATOR);
    let ct_bytes =
        erasure::decode(ct_shards, kem::CT_LEN, n_original, n_recovery)?;

    if ct_bytes.len() != kem::CT_LEN {
        return Err(RatchetError::BraidChunkReassembly { epoch });
    }
    let ct: [u8; kem::CT_LEN] = ct_bytes
        .try_into()
        .map_err(|_| RatchetError::BraidChunkReassembly { epoch })?;

    let ss = kem::decaps(dk, &ct)?;

    Ok(MlKemBraidState::Complete { epoch, epoch_ss: ss })
}

/// Determine how many original shards are needed for ek_vector decode.
pub fn ek_vector_original_count() -> usize {
    EK_VECTOR_LEN.div_ceil(erasure::DATA_CHUNK_SIZE)
}

/// Determine how many original shards are needed for ciphertext decode.
pub fn ciphertext_original_count() -> usize {
    kem::CT_LEN.div_ceil(erasure::DATA_CHUNK_SIZE)
}

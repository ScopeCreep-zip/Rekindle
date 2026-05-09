//! Reed-Solomon erasure coding for SPQR chunk transport.
//!
//! Parameters (Option A from spec): 32-byte data chunks padded to
//! 64-byte shards for `reed-solomon-simd` compatibility.
//!
//! ML-KEM-768 chunk counts:
//! - Header (ek_seed || hek): 64 bytes → 1 shard, no RS
//! - ek_vector: 1152 bytes → 36 original + 9 recovery = 45 shards
//! - Ciphertext: 1088 bytes → 34 original + 9 recovery = 43 shards
//! - Total per epoch: 1 + 45 + 43 = 89 shards

use crate::error::RatchetError;

pub const DATA_CHUNK_SIZE: usize = 32;
pub const SHARD_SIZE: usize = 64;
pub const RECOVERY_RATIO_NUMERATOR: usize = 1;
pub const RECOVERY_RATIO_DENOMINATOR: usize = 4;

/// Chunk kind for SPQR transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkKind {
    Header,
    EkVector,
    Ciphertext,
}

/// A single shard ready for transport.
#[derive(Debug, Clone)]
pub struct Shard {
    pub kind: ChunkKind,
    pub epoch: u32,
    pub index: u32,
    pub is_recovery: bool,
    pub data: [u8; SHARD_SIZE],
}

/// Encode data into original + recovery shards.
pub fn encode(
    data: &[u8],
    kind: ChunkKind,
    epoch: u32,
) -> Result<Vec<Shard>, RatchetError> {
    if matches!(kind, ChunkKind::Header) {
        // Header is a single shard, no RS
        let mut shard_data = [0u8; SHARD_SIZE];
        let copy_len = data.len().min(SHARD_SIZE);
        shard_data[..copy_len].copy_from_slice(&data[..copy_len]);
        return Ok(vec![Shard {
            kind,
            epoch,
            index: 0,
            is_recovery: false,
            data: shard_data,
        }]);
    }

    let n_original = data.len().div_ceil(DATA_CHUNK_SIZE);
    let n_recovery = (n_original * RECOVERY_RATIO_NUMERATOR).div_ceil(RECOVERY_RATIO_DENOMINATOR);

    let mut encoder =
        reed_solomon_simd::ReedSolomonEncoder::new(n_original, n_recovery, SHARD_SIZE)
            .map_err(|e| RatchetError::ErasureEncode(format!("{e}")))?;

    // Add original shards (32-byte data padded to 64 bytes)
    for i in 0..n_original {
        let mut shard = [0u8; SHARD_SIZE];
        let start = i * DATA_CHUNK_SIZE;
        let end = (start + DATA_CHUNK_SIZE).min(data.len());
        shard[..end - start].copy_from_slice(&data[start..end]);
        encoder
            .add_original_shard(shard)
            .map_err(|e| RatchetError::ErasureEncode(format!("{e}")))?;
    }

    let result = encoder
        .encode()
        .map_err(|e| RatchetError::ErasureEncode(format!("{e}")))?;

    let mut shards = Vec::with_capacity(n_original + n_recovery);

    // Original shards
    for i in 0..n_original {
        let mut shard_data = [0u8; SHARD_SIZE];
        let start = i * DATA_CHUNK_SIZE;
        let end = (start + DATA_CHUNK_SIZE).min(data.len());
        shard_data[..end - start].copy_from_slice(&data[start..end]);
        shards.push(Shard {
            kind,
            epoch,
            index: u32::try_from(i).unwrap_or(u32::MAX),
            is_recovery: false,
            data: shard_data,
        });
    }

    // Recovery shards
    for (j, rec) in result.recovery_iter().enumerate() {
        let mut shard_data = [0u8; SHARD_SIZE];
        shard_data.copy_from_slice(rec);
        shards.push(Shard {
            kind,
            epoch,
            index: u32::try_from(n_original + j).unwrap_or(u32::MAX),
            is_recovery: true,
            data: shard_data,
        });
    }

    Ok(shards)
}

/// Decode shards back into the original data.
///
/// Requires at least `n_original` shards (original or recovery).
pub fn decode(
    shards: &[Shard],
    expected_len: usize,
    n_original: usize,
    n_recovery: usize,
) -> Result<Vec<u8>, RatchetError> {
    let mut decoder =
        reed_solomon_simd::ReedSolomonDecoder::new(n_original, n_recovery, SHARD_SIZE)
            .map_err(|_| RatchetError::ErasureDecode)?;

    for shard in shards {
        let idx = shard.index as usize;
        if shard.is_recovery {
            let recovery_idx = idx.checked_sub(n_original).ok_or(RatchetError::ErasureDecode)?;
            decoder
                .add_recovery_shard(recovery_idx, shard.data)
                .map_err(|_| RatchetError::ErasureDecode)?;
        } else {
            decoder
                .add_original_shard(idx, shard.data)
                .map_err(|_| RatchetError::ErasureDecode)?;
        }
    }

    let result = decoder.decode().map_err(|_| RatchetError::ErasureDecode)?;

    // Reassemble original data from decoded shards
    let mut out = vec![0u8; n_original * DATA_CHUNK_SIZE];
    for (idx, restored) in result.restored_original_iter() {
        let start = idx * DATA_CHUNK_SIZE;
        let end = (start + DATA_CHUNK_SIZE).min(out.len());
        out[start..end].copy_from_slice(&restored[..end - start]);
    }

    // Also fill in shards we already had
    for shard in shards {
        if !shard.is_recovery {
            let start = shard.index as usize * DATA_CHUNK_SIZE;
            let end = (start + DATA_CHUNK_SIZE).min(out.len());
            out[start..end].copy_from_slice(&shard.data[..end - start]);
        }
    }

    out.truncate(expected_len);
    Ok(out)
}

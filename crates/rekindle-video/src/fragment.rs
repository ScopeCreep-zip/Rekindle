//! Architecture §10.6 video fragmentation. A single encoded video
//! frame may exceed Veilid's `app_message` 32 KiB cap, so we split it
//! into ≤28 KiB chunks (the §10.6 budget — 28 KiB leaves room for the
//! envelope, signature, MEK overhead, and Cap'n Proto framing).

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Per-fragment payload limit. Spec line 3231 ("Veilid `app_message`
/// payload limit minus overhead") for files; the same budget applies
/// to video fragments.
pub const FRAGMENT_PAYLOAD_LIMIT: usize = 28 * 1024;

/// Maximum fragments per frame. `frag_total: u8` per the spec's
/// `VideoFragment` struct caps us at 256 — well above any frame we'd
/// realistically ship at 480p @ 15 fps.
pub const MAX_FRAGMENTS_PER_FRAME: usize = 256;

/// 16-byte stream identifier — derived from `(channel_id || sender_pseudonym)`
/// so concurrent streams (e.g. two members screen-sharing in the same
/// channel) don't collide.
pub const STREAM_ID_LEN: usize = 16;

/// On-the-wire fragment matching architecture §10.6 line 2062.
/// Carries one chunk of an MEK-encrypted, VP9-encoded video frame.
/// FEC parity packets travel as a separate [`VideoParityFragment`]
/// variant so this type stays canonical to the spec.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoFragment {
    pub stream_id: [u8; STREAM_ID_LEN],
    /// Monotonic frame counter assigned by the sender.
    pub frame_seq: u32,
    /// 0-based fragment index within the frame, in `[0, frag_total)`.
    pub frag_index: u8,
    /// Total number of source (data) fragments the frame was split
    /// into. Parity fragments live in a separate stream and don't
    /// count here.
    pub frag_total: u8,
    /// True for keyframes (I-frames). Receivers without one drop
    /// inter-frames until the next keyframe arrives.
    pub keyframe: bool,
    /// Sender wall-clock at frame capture (ms since epoch, truncated
    /// to u32 — drift across ~50 days is acceptable for a streaming
    /// protocol where freshness is local-relative).
    pub timestamp: u32,
    /// MEK-encrypted fragment payload. Per architecture §10.6 line 2057
    /// the MEK encryption happens before fragmentation.
    pub payload: Vec<u8>,
    /// Ed25519 signature over `fragment_signing_bytes(...)`.
    pub signature: Vec<u8>,
}

/// Parity (forward-error-correction) fragment for a single video
/// frame. Architecture §10.6 line 4080 calls for "FEC data" on
/// `VideoFragment`; we ship it as a sibling variant — modelled on
/// RFC 5109 / FlexFEC's separate FEC packet stream — so the
/// canonical `VideoFragment` shape stays per spec and receivers
/// without FEC support can ignore parity packets harmlessly.
///
/// One parity fragment covers `data_count` consecutive source
/// fragments of a single `frame_seq`. The Reed-Solomon code (n=K+M)
/// lets the receiver reconstruct the original frame from any
/// `data_count` of the `data_count + parity_total` fragments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoParityFragment {
    pub stream_id: [u8; STREAM_ID_LEN],
    pub frame_seq: u32,
    /// 0-based parity index, in `[0, parity_total)`.
    pub parity_index: u8,
    /// Total number of parity fragments shipped for this frame.
    pub parity_total: u8,
    /// Number of source (data) fragments this parity set covers —
    /// matches the data fragments' `frag_total`.
    pub data_count: u8,
    /// Original encrypted-frame length in bytes. Reed-Solomon shards
    /// must be equal-size, so the last data shard is null-padded
    /// during encode; the receiver truncates to `frame_len` after
    /// reconstruction.
    pub frame_len: u32,
    pub timestamp: u32,
    /// MEK-encrypted parity bytes — same shard size as the data
    /// fragments' payload (i.e. `ceil(frame_len / data_count)`).
    pub payload: Vec<u8>,
    /// Ed25519 signature over `parity_signing_bytes(...)`.
    pub signature: Vec<u8>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FragmentError {
    #[error("frame is empty")]
    EmptyFrame,
    #[error("frame would split into {0} fragments — exceeds MAX_FRAGMENTS_PER_FRAME = {MAX_FRAGMENTS_PER_FRAME}")]
    TooManyFragments(usize),
    #[error("parity_count must be > 0 for fragment_with_fec; use fragment_frame for non-FEC")]
    ZeroParity,
    #[error("Reed-Solomon encode failed: {0}")]
    Fec(String),
}

/// Canonical bytes-to-sign for one fragment, per architecture §10.6
/// line 2071: `(stream_id || frame_seq || frag_index || frag_total ||
/// keyframe || timestamp || payload)`. Exposed so the orchestrator
/// in `src-tauri` (which holds the signing key) can sign before
/// dispatch without re-implementing the byte layout.
pub fn fragment_signing_bytes(fragment: &VideoFragment) -> Vec<u8> {
    let mut buf =
        Vec::with_capacity(STREAM_ID_LEN + 4 + 1 + 1 + 1 + 4 + fragment.payload.len());
    buf.extend_from_slice(&fragment.stream_id);
    buf.extend_from_slice(&fragment.frame_seq.to_le_bytes());
    buf.push(fragment.frag_index);
    buf.push(fragment.frag_total);
    buf.push(u8::from(fragment.keyframe));
    buf.extend_from_slice(&fragment.timestamp.to_le_bytes());
    buf.extend_from_slice(&fragment.payload);
    buf
}

/// Canonical bytes-to-sign for one parity fragment. Mirrors
/// `fragment_signing_bytes` but covers the FEC-specific fields.
pub fn parity_signing_bytes(fragment: &VideoParityFragment) -> Vec<u8> {
    let mut buf =
        Vec::with_capacity(STREAM_ID_LEN + 4 + 1 + 1 + 1 + 4 + 4 + fragment.payload.len());
    buf.extend_from_slice(&fragment.stream_id);
    buf.extend_from_slice(&fragment.frame_seq.to_le_bytes());
    buf.push(fragment.parity_index);
    buf.push(fragment.parity_total);
    buf.push(fragment.data_count);
    buf.extend_from_slice(&fragment.frame_len.to_le_bytes());
    buf.extend_from_slice(&fragment.timestamp.to_le_bytes());
    buf.extend_from_slice(&fragment.payload);
    buf
}

/// Split an already-encrypted frame into ≤`FRAGMENT_PAYLOAD_LIMIT`
/// chunks. Caller signs each fragment afterwards using the sender's
/// pseudonym key — this module is sign-agnostic.
pub fn fragment_frame(
    stream_id: [u8; STREAM_ID_LEN],
    frame_seq: u32,
    keyframe: bool,
    timestamp: u32,
    encrypted_frame: &[u8],
) -> Result<Vec<VideoFragment>, FragmentError> {
    if encrypted_frame.is_empty() {
        return Err(FragmentError::EmptyFrame);
    }
    let total = encrypted_frame.len().div_ceil(FRAGMENT_PAYLOAD_LIMIT);
    if total > MAX_FRAGMENTS_PER_FRAME {
        return Err(FragmentError::TooManyFragments(total));
    }
    let total_u8 = u8::try_from(total).expect("checked above");

    let mut fragments = Vec::with_capacity(total);
    for (idx, chunk) in encrypted_frame.chunks(FRAGMENT_PAYLOAD_LIMIT).enumerate() {
        fragments.push(VideoFragment {
            stream_id,
            frame_seq,
            frag_index: u8::try_from(idx).expect("idx <= total_u8 - 1 <= u8::MAX"),
            frag_total: total_u8,
            keyframe,
            timestamp,
            payload: chunk.to_vec(),
            signature: Vec::new(),
        });
    }
    Ok(fragments)
}

/// Result of `fragment_frame_with_fec`: the spec-shaped data fragments
/// plus the matching parity fragments. Caller signs both lists with
/// the sender's pseudonym key before dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FecFragments {
    pub data: Vec<VideoFragment>,
    pub parity: Vec<VideoParityFragment>,
}

/// Fragment a frame into `data + parity` shards using Reed-Solomon
/// erasure coding. Receivers can reconstruct the frame from any
/// `data.len()` of the `data.len() + parity.len()` total shards.
///
/// All shards must be equal-size for Reed-Solomon, so the encrypted
/// frame is null-padded to a multiple of the per-shard size before
/// encoding. The original length travels in each parity fragment's
/// `frame_len` field for receiver-side truncation.
pub fn fragment_frame_with_fec(
    stream_id: [u8; STREAM_ID_LEN],
    frame_seq: u32,
    keyframe: bool,
    timestamp: u32,
    encrypted_frame: &[u8],
    parity_count: u8,
) -> Result<FecFragments, FragmentError> {
    use reed_solomon_erasure::galois_8::ReedSolomon;

    if encrypted_frame.is_empty() {
        return Err(FragmentError::EmptyFrame);
    }
    if parity_count == 0 {
        return Err(FragmentError::ZeroParity);
    }

    let frame_len = u32::try_from(encrypted_frame.len()).unwrap_or(u32::MAX);
    let data_count = encrypted_frame.len().div_ceil(FRAGMENT_PAYLOAD_LIMIT);
    if data_count + usize::from(parity_count) > MAX_FRAGMENTS_PER_FRAME {
        return Err(FragmentError::TooManyFragments(
            data_count + usize::from(parity_count),
        ));
    }
    let data_count_u8 = u8::try_from(data_count).expect("checked above");
    let shard_size = encrypted_frame.len().div_ceil(data_count);

    // Build equal-sized data shards, null-padding the last one.
    let mut shards: Vec<Vec<u8>> = Vec::with_capacity(data_count + usize::from(parity_count));
    for chunk in encrypted_frame.chunks(shard_size) {
        let mut shard = chunk.to_vec();
        shard.resize(shard_size, 0);
        shards.push(shard);
    }
    for _ in 0..parity_count {
        shards.push(vec![0u8; shard_size]);
    }

    let rs = ReedSolomon::new(data_count, usize::from(parity_count))
        .map_err(|e| FragmentError::Fec(format!("rs init: {e}")))?;
    rs.encode(&mut shards)
        .map_err(|e| FragmentError::Fec(format!("rs encode: {e}")))?;

    let mut data: Vec<VideoFragment> = Vec::with_capacity(data_count);
    for (idx, shard) in shards.iter().take(data_count).enumerate() {
        data.push(VideoFragment {
            stream_id,
            frame_seq,
            frag_index: u8::try_from(idx).expect("data_count fits u8"),
            frag_total: data_count_u8,
            keyframe,
            timestamp,
            payload: shard.clone(),
            signature: Vec::new(),
        });
    }
    let mut parity: Vec<VideoParityFragment> = Vec::with_capacity(usize::from(parity_count));
    for (idx, shard) in shards.iter().skip(data_count).enumerate() {
        parity.push(VideoParityFragment {
            stream_id,
            frame_seq,
            parity_index: u8::try_from(idx).expect("parity_count is u8"),
            parity_total: parity_count,
            data_count: data_count_u8,
            frame_len,
            timestamp,
            payload: shard.clone(),
            signature: Vec::new(),
        });
    }
    Ok(FecFragments { data, parity })
}

/// Reconstruct the original encrypted frame from any subset of
/// `(data_count + parity_total)` fragments containing at least
/// `data_count` shards. Returns the ciphertext bytes (caller MEK-
/// decrypts).
///
/// `received_data` is `(frag_index, payload)` pairs; `received_parity`
/// is `(parity_index, payload)`. `data_count`, `parity_total`, `frame_len`
/// must match what the sender shipped (they're carried on every
/// parity fragment, so a single received parity is enough to fill them).
pub fn reconstruct_frame(
    received_data: &[(u8, Vec<u8>)],
    received_parity: &[(u8, Vec<u8>)],
    data_count: u8,
    parity_total: u8,
    frame_len: u32,
) -> Result<Vec<u8>, FragmentError> {
    use reed_solomon_erasure::galois_8::ReedSolomon;

    if data_count == 0 {
        return Err(FragmentError::EmptyFrame);
    }
    let total = usize::from(data_count) + usize::from(parity_total);
    let received_total = received_data.len() + received_parity.len();
    if received_total < usize::from(data_count) {
        return Err(FragmentError::Fec(format!(
            "need {data_count} shards, got {received_total}"
        )));
    }

    // Determine shard size from any received payload — they're all equal.
    let shard_size = received_data
        .first()
        .map(|(_, p)| p.len())
        .or_else(|| received_parity.first().map(|(_, p)| p.len()))
        .unwrap_or(0);
    if shard_size == 0 {
        return Err(FragmentError::EmptyFrame);
    }

    let mut shards: Vec<Option<Vec<u8>>> = vec![None; total];
    for (idx, payload) in received_data {
        let i = usize::from(*idx);
        if i < usize::from(data_count) && payload.len() == shard_size {
            shards[i] = Some(payload.clone());
        }
    }
    for (idx, payload) in received_parity {
        let i = usize::from(data_count) + usize::from(*idx);
        if i < total && payload.len() == shard_size {
            shards[i] = Some(payload.clone());
        }
    }

    let rs = ReedSolomon::new(usize::from(data_count), usize::from(parity_total))
        .map_err(|e| FragmentError::Fec(format!("rs init: {e}")))?;
    rs.reconstruct_data(&mut shards)
        .map_err(|e| FragmentError::Fec(format!("rs reconstruct: {e}")))?;

    let mut out = Vec::with_capacity(usize::from(data_count) * shard_size);
    for shard in shards.iter().take(usize::from(data_count)).flatten() {
        out.extend_from_slice(shard);
    }
    out.truncate(frame_len as usize);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_frame_rejected() {
        let err = fragment_frame([0u8; STREAM_ID_LEN], 1, true, 0, &[]).unwrap_err();
        assert_eq!(err, FragmentError::EmptyFrame);
    }

    #[test]
    fn small_frame_one_fragment() {
        let frame = vec![0xABu8; 1024];
        let frags = fragment_frame([1u8; STREAM_ID_LEN], 7, true, 100, &frame).unwrap();
        assert_eq!(frags.len(), 1);
        assert_eq!(frags[0].frame_seq, 7);
        assert!(frags[0].keyframe);
        assert_eq!(frags[0].frag_total, 1);
        assert_eq!(frags[0].payload, frame);
    }

    #[test]
    fn large_frame_multiple_fragments() {
        let frame = vec![0x55u8; FRAGMENT_PAYLOAD_LIMIT * 3 + 100];
        let frags = fragment_frame([2u8; STREAM_ID_LEN], 12, false, 200, &frame).unwrap();
        assert_eq!(frags.len(), 4);
        assert_eq!(frags[3].frag_index, 3);
        assert_eq!(frags[3].frag_total, 4);
        // Concatenating the payloads must reproduce the original bytes.
        let reassembled: Vec<u8> = frags.iter().flat_map(|f| f.payload.clone()).collect();
        assert_eq!(reassembled, frame);
    }

    #[test]
    fn fec_encode_then_reconstruct_when_no_drops() {
        let frame = vec![0xAAu8; FRAGMENT_PAYLOAD_LIMIT * 3 + 100];
        let frags = fragment_frame_with_fec(
            [9u8; STREAM_ID_LEN],
            42,
            true,
            500,
            &frame,
            2,
        )
        .unwrap();
        assert_eq!(frags.data.len(), 4);
        assert_eq!(frags.parity.len(), 2);

        let received_data: Vec<(u8, Vec<u8>)> = frags
            .data
            .iter()
            .map(|f| (f.frag_index, f.payload.clone()))
            .collect();
        let received_parity: Vec<(u8, Vec<u8>)> = frags
            .parity
            .iter()
            .map(|f| (f.parity_index, f.payload.clone()))
            .collect();
        let recovered = reconstruct_frame(
            &received_data,
            &received_parity,
            4,
            2,
            u32::try_from(frame.len()).unwrap(),
        )
        .unwrap();
        assert_eq!(recovered, frame);
    }

    #[test]
    fn fec_recovers_from_two_dropped_data_shards() {
        // 4 data shards + 2 parity. Drop 2 data shards. Reconstruct
        // from remaining 2 data + 2 parity = 4 total ≥ 4 needed.
        let frame: Vec<u8> = (0..FRAGMENT_PAYLOAD_LIMIT * 3 + 7)
            .map(|i| u8::try_from(i & 0xff).unwrap())
            .collect();
        let frags = fragment_frame_with_fec(
            [3u8; STREAM_ID_LEN],
            7,
            true,
            100,
            &frame,
            2,
        )
        .unwrap();

        // Drop frag_index 1 and 3.
        let received_data: Vec<(u8, Vec<u8>)> = frags
            .data
            .iter()
            .filter(|f| f.frag_index != 1 && f.frag_index != 3)
            .map(|f| (f.frag_index, f.payload.clone()))
            .collect();
        let received_parity: Vec<(u8, Vec<u8>)> = frags
            .parity
            .iter()
            .map(|f| (f.parity_index, f.payload.clone()))
            .collect();
        let recovered = reconstruct_frame(
            &received_data,
            &received_parity,
            4,
            2,
            u32::try_from(frame.len()).unwrap(),
        )
        .unwrap();
        assert_eq!(recovered, frame);
    }

    #[test]
    fn fec_fails_when_too_many_shards_dropped() {
        // 4 data + 2 parity. Drop 3 data + 1 parity. Only 2 shards
        // remain — below the 4-shard threshold for reconstruction.
        let frame = vec![0x77u8; FRAGMENT_PAYLOAD_LIMIT * 3 + 50];
        let frags = fragment_frame_with_fec(
            [4u8; STREAM_ID_LEN],
            8,
            true,
            200,
            &frame,
            2,
        )
        .unwrap();
        let received_data: Vec<(u8, Vec<u8>)> = frags
            .data
            .iter()
            .filter(|f| f.frag_index == 0)
            .map(|f| (f.frag_index, f.payload.clone()))
            .collect();
        let received_parity: Vec<(u8, Vec<u8>)> = frags
            .parity
            .iter()
            .filter(|f| f.parity_index == 0)
            .map(|f| (f.parity_index, f.payload.clone()))
            .collect();
        let err = reconstruct_frame(
            &received_data,
            &received_parity,
            4,
            2,
            u32::try_from(frame.len()).unwrap(),
        )
        .unwrap_err();
        match err {
            FragmentError::Fec(_) => {}
            other => panic!("expected Fec error, got {other:?}"),
        }
    }

    #[test]
    fn fec_zero_parity_rejected() {
        let frame = vec![0u8; 100];
        let err =
            fragment_frame_with_fec([0u8; STREAM_ID_LEN], 1, true, 0, &frame, 0).unwrap_err();
        assert_eq!(err, FragmentError::ZeroParity);
    }

    #[test]
    fn parity_signing_bytes_layout() {
        let p = VideoParityFragment {
            stream_id: [0xCDu8; STREAM_ID_LEN],
            frame_seq: 0x0102_0304,
            parity_index: 1,
            parity_total: 2,
            data_count: 4,
            frame_len: 1234,
            timestamp: 0xCAFE_BABE,
            payload: vec![0x10, 0x20],
            signature: Vec::new(),
        };
        let bytes = parity_signing_bytes(&p);
        assert_eq!(&bytes[..STREAM_ID_LEN], &[0xCDu8; STREAM_ID_LEN]);
        assert_eq!(&bytes[STREAM_ID_LEN..STREAM_ID_LEN + 4], &0x0102_0304u32.to_le_bytes());
        assert_eq!(bytes[STREAM_ID_LEN + 4], 1);
        assert_eq!(bytes[STREAM_ID_LEN + 5], 2);
        assert_eq!(bytes[STREAM_ID_LEN + 6], 4);
        assert_eq!(&bytes[STREAM_ID_LEN + 7..STREAM_ID_LEN + 11], &1234u32.to_le_bytes());
        assert_eq!(
            &bytes[STREAM_ID_LEN + 11..STREAM_ID_LEN + 15],
            &0xCAFE_BABEu32.to_le_bytes()
        );
        assert_eq!(&bytes[STREAM_ID_LEN + 15..], &[0x10, 0x20]);
    }

    #[test]
    fn signing_bytes_match_spec_layout() {
        // Architecture §10.6 line 2071 — the canonical bytes-to-sign
        // are `(stream_id || frame_seq || frag_index || frag_total ||
        // keyframe || timestamp || payload)`.
        let frag = VideoFragment {
            stream_id: [0xABu8; STREAM_ID_LEN],
            frame_seq: 0x1122_3344,
            frag_index: 2,
            frag_total: 5,
            keyframe: true,
            timestamp: 0xDEAD_BEEF,
            payload: vec![0x01, 0x02, 0x03],
            signature: Vec::new(),
        };
        let bytes = fragment_signing_bytes(&frag);
        assert_eq!(&bytes[..STREAM_ID_LEN], &[0xABu8; STREAM_ID_LEN]);
        assert_eq!(&bytes[STREAM_ID_LEN..STREAM_ID_LEN + 4], &0x1122_3344u32.to_le_bytes());
        assert_eq!(bytes[STREAM_ID_LEN + 4], 2);
        assert_eq!(bytes[STREAM_ID_LEN + 5], 5);
        assert_eq!(bytes[STREAM_ID_LEN + 6], 1);
        assert_eq!(
            &bytes[STREAM_ID_LEN + 7..STREAM_ID_LEN + 11],
            &0xDEAD_BEEFu32.to_le_bytes()
        );
        assert_eq!(&bytes[STREAM_ID_LEN + 11..], &[0x01, 0x02, 0x03]);
    }

    #[test]
    fn rejects_frame_exceeding_max_fragments() {
        let frame = vec![0u8; FRAGMENT_PAYLOAD_LIMIT * (MAX_FRAGMENTS_PER_FRAME + 1)];
        let err = fragment_frame([3u8; STREAM_ID_LEN], 1, true, 0, &frame).unwrap_err();
        match err {
            FragmentError::TooManyFragments(n) => {
                assert!(n > MAX_FRAGMENTS_PER_FRAME);
            }
            _ => panic!("wrong error variant"),
        }
    }
}

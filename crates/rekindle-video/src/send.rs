//! Phase 16 — community video send pipeline (build side).
//!
//! Architecture §10.6 — MEK-encrypt the encoded payload, fragment to
//! the 4 KiB transport budget, sign each fragment with the community
//! pseudonym Ed25519
//! key. The signed envelopes are NOT dispatched here: they are returned
//! as one [`BuiltVideoFrame`], which the caller queues on each roster
//! peer's route, behind audio (plan E4.3.3) — never on the community
//! gossip mesh.
//!
//! The reassembly state is consulted ONLY to fire a one-shot
//! `TopologyChange { reason: "initial" }` per (community, stream) so
//! receivers know to spin up a decoder; that tiny control envelope is
//! still sent IMMEDIATELY (it must precede the first fragment).

use rekindle_codec::community::envelope::{
    CommunityEnvelope, ControlPayload, VideoFragmentPayload, VideoParityFragmentPayload,
};
use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
use rekindle_types::video::Codec;

use crate::deps::VideoDeps;
use crate::error::VideoError;
use crate::fragment::{
    fragment_frame, fragment_frame_with_fec, fragment_signing_bytes, parity_signing_bytes,
    FRAGMENT_PAYLOAD_LIMIT,
};
use crate::reassembly_state::VideoReassemblyState;

/// One parity per N data shards for any multi-fragment frame. With 4×
/// ratio, dropping up to 25% of fragments is recoverable. The old
/// keyframes-only gate predates the 4 KiB fragment budget: a lost
/// delta fragment now costs a keyframe request (a full intra on the
/// wire), which is far more expensive than 25% parity on the delta.
const PARITY_RATIO_DENOM: usize = 4;

/// One encoded frame, encrypted, fragmented and signed, ready for every
/// peer's route.
#[derive(Debug, Clone)]
pub struct BuiltVideoFrame {
    pub stream_id: [u8; 16],
    pub frame_seq: u32,
    pub keyframe: bool,
    /// Signed data + parity fragment envelopes, in send order.
    pub envelopes: Vec<CommunityEnvelope>,
    /// Encoder output bytes the envelopes carry.
    pub media_bytes: usize,
}

/// Per-frame send request. Bundling all the variable-per-frame fields
/// into a struct keeps the orchestration helpers below a sane argument
/// count and gives the Tauri command a clear input shape.
#[derive(Debug)]
pub struct VideoFrameSend {
    pub stream_id: [u8; 16],
    pub frame_seq: u32,
    pub keyframe: bool,
    /// Codec the frontend encoder produced this chunk with — travels
    /// on every fragment (RTP payload-type analog) and is covered by
    /// the fragment signature.
    pub codec: Codec,
    pub timestamp: u32,
    pub encoded_payload: Vec<u8>,
}

/// Build-side entry point — invoked from the `send_video_frame` Tauri
/// command after the webview encoder produces a `VideoEncoder.encode()`
/// chunk. MEK-encrypts the payload, fragments to the transport budget
/// (`FRAGMENT_PAYLOAD_LIMIT`, 4 KiB), signs each
/// fragment with the sender's pseudonym Ed25519 key, and returns the
/// envelopes as one [`BuiltVideoFrame`].
pub fn build_video_frame<D: VideoDeps>(
    deps: &D,
    reassembly: &VideoReassemblyState,
    community_id: &str,
    channel_id: &str,
    request: &VideoFrameSend,
) -> Result<BuiltVideoFrame, VideoError> {
    // Phase F — IPC entry trace. The Tauri command in src-tauri delivered
    // an encoded VP9 chunk from the WebView; record the byte count and
    // routing context (no payload bytes) so a `RUST_LOG=rekindle_video=
    // debug` operator can confirm the chunk arrived from the frontend
    // before encrypt + fragment + sign run.
    tracing::debug!(
        target: "rekindle_video::send",
        community_id = %community_id,
        channel_id = %channel_id,
        stream_id = %hex::encode(request.stream_id),
        frame_seq = request.frame_seq,
        keyframe = request.keyframe,
        encoded_bytes = request.encoded_payload.len(),
        "received encoded frame from frontend"
    );
    if request.encoded_payload.is_empty() {
        return Err(VideoError::InvalidInput("empty encoded payload".into()));
    }

    let signing_key = deps
        .community_signing_key(community_id)
        .ok_or(VideoError::IdentityNotLoaded)?;
    // Our own media key (plan C7.20): the one voice seals under too, sent
    // to every participant we hold on the roster.
    let key = deps
        .channel_sender_keys(community_id, channel_id)
        .send_key(std::time::Instant::now());
    let frame_key = rekindle_secrets::media_sender_key::video_frame_key(
        &key.secret,
        &signing_key.verifying_key().to_bytes(),
    );
    let ciphertext =
        rekindle_crypto::group::media_key::MediaEncryptionKey::from_bytes(*frame_key, key.index)
            .encrypt(&request.encoded_payload)
            .map_err(|e| VideoError::Encrypt(format!("frame encrypt: {e}")))?;

    let ctx = SendCtx {
        channel_id,
        stream_id: request.stream_id,
        frame_seq: request.frame_seq,
        keyframe: request.keyframe,
        codec: request.codec,
        timestamp: request.timestamp,
        key_index: key.index,
        signing_key: &signing_key,
    };

    // Architecture §32 Phase 6 Week 22 — emit a `TopologyChange { reason
    // = "initial" }` exactly once per stream so receivers know to spin
    // up a decoder. `mark_stream_started` returns true exactly once per
    // (community, stream) since startup.
    if reassembly.mark_stream_started(community_id, request.stream_id) {
        emit_initial_topology(deps, community_id, channel_id, request.stream_id)?;
    }

    let parity_count = parity_count_for(&ciphertext);
    let envelopes = if parity_count > 0 {
        ctx.collect_with_fec(&ciphertext, parity_count)?
    } else {
        ctx.collect_without_fec(&ciphertext)?
    };
    Ok(BuiltVideoFrame {
        stream_id: request.stream_id,
        frame_seq: request.frame_seq,
        keyframe: request.keyframe,
        envelopes,
        media_bytes: request.encoded_payload.len(),
    })
}

fn emit_initial_topology<D: VideoDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    stream_id: [u8; 16],
) -> Result<(), VideoError> {
    let lamport = deps.increment_lamport(community_id)?;
    let envelope = CommunityEnvelope::Control(ControlPayload::TopologyChange {
        channel_id: channel_id.to_string(),
        stream_id,
        // Initial topology is full-mesh broadcast — no relay host yet.
        // Future: when relay-peer routing lands for video, the elected
        // relay's pseudonym goes here and senders adjust their dispatch.
        relay_host_pseudonym: None,
        reason: "initial".to_string(),
        lamport,
    });
    deps.send_to_channel(community_id, channel_id, &envelope)
}

#[must_use]
fn parity_count_for(ciphertext: &[u8]) -> u8 {
    let data = ciphertext.len().div_ceil(FRAGMENT_PAYLOAD_LIMIT);
    if data < 2 {
        // 1-shard frames don't benefit from parity (parity = duplicate)
        // — and reed-solomon over 1+1 only recovers exact duplicates.
        return 0;
    }
    u8::try_from(data.div_ceil(PARITY_RATIO_DENOM)).unwrap_or(u8::MAX)
}

/// Bundle of references the FEC and non-FEC collect helpers both
/// need. Keeps each helper at one parameter (`ciphertext`) plus the
/// shared context.
struct SendCtx<'a> {
    channel_id: &'a str,
    stream_id: [u8; 16],
    frame_seq: u32,
    keyframe: bool,
    codec: Codec,
    timestamp: u32,
    /// Index of our media sender key that encrypted the frame (plan
    /// C7.20) — rides every fragment so receivers can request that key.
    key_index: u64,
    signing_key: &'a SigningKey,
}

impl SendCtx<'_> {
    fn frame_shape(&self) -> crate::fragment::FrameShape {
        crate::fragment::FrameShape {
            stream_id: self.stream_id,
            frame_seq: self.frame_seq,
            keyframe: self.keyframe,
            codec: self.codec,
            timestamp: self.timestamp,
            key_index: self.key_index,
        }
    }
}

impl SendCtx<'_> {
    fn collect_without_fec(&self, ciphertext: &[u8]) -> Result<Vec<CommunityEnvelope>, VideoError> {
        let mut fragments = fragment_frame(self.frame_shape(), ciphertext)?;
        let count = u32::try_from(fragments.len()).unwrap_or(u32::MAX);
        for fragment in &mut fragments {
            let to_sign = fragment_signing_bytes(fragment);
            fragment.signature = self.signing_key.sign(&to_sign).to_bytes().to_vec();
        }
        let total_bytes: usize = fragments.iter().map(|f| f.payload.len()).sum();
        let envelopes: Vec<CommunityEnvelope> = fragments
            .into_iter()
            .map(|fragment| {
                CommunityEnvelope::Control(ControlPayload::VideoFragment(VideoFragmentPayload {
                    channel_id: self.channel_id.to_string(),
                    stream_id: fragment.stream_id,
                    frame_seq: fragment.frame_seq,
                    frag_index: fragment.frag_index,
                    frag_total: fragment.frag_total,
                    keyframe: fragment.keyframe,
                    codec: fragment.codec,
                    timestamp: fragment.timestamp,
                    key_index: fragment.key_index,
                    payload: fragment.payload,
                    signature: fragment.signature,
                }))
            })
            .collect();
        tracing::debug!(
            target: "rekindle_video::send",
            frame_seq = self.frame_seq,
            fragment_count = count,
            total_bytes = total_bytes,
            stream_id = %hex::encode(self.stream_id),
            keyframe = self.keyframe,
            "built video frame"
        );
        Ok(envelopes)
    }

    fn collect_with_fec(
        &self,
        ciphertext: &[u8],
        parity_count: u8,
    ) -> Result<Vec<CommunityEnvelope>, VideoError> {
        let mut fec = fragment_frame_with_fec(self.frame_shape(), ciphertext, parity_count)?;

        for fragment in &mut fec.data {
            let to_sign = fragment_signing_bytes(fragment);
            fragment.signature = self.signing_key.sign(&to_sign).to_bytes().to_vec();
        }
        for fragment in &mut fec.parity {
            let to_sign = parity_signing_bytes(fragment);
            fragment.signature = self.signing_key.sign(&to_sign).to_bytes().to_vec();
        }

        let total = u32::try_from(fec.data.len() + fec.parity.len()).unwrap_or(u32::MAX);
        let data_count = u32::try_from(fec.data.len()).unwrap_or(u32::MAX);
        let parity_count_total = u32::try_from(fec.parity.len()).unwrap_or(u32::MAX);
        let data_bytes: usize = fec.data.iter().map(|f| f.payload.len()).sum();
        let parity_bytes: usize = fec.parity.iter().map(|f| f.payload.len()).sum();
        let mut envelopes: Vec<CommunityEnvelope> =
            Vec::with_capacity(usize::try_from(total).unwrap_or(0));
        for fragment in fec.data {
            envelopes.push(CommunityEnvelope::Control(ControlPayload::VideoFragment(
                VideoFragmentPayload {
                    channel_id: self.channel_id.to_string(),
                    stream_id: fragment.stream_id,
                    frame_seq: fragment.frame_seq,
                    frag_index: fragment.frag_index,
                    frag_total: fragment.frag_total,
                    keyframe: fragment.keyframe,
                    codec: fragment.codec,
                    timestamp: fragment.timestamp,
                    key_index: fragment.key_index,
                    payload: fragment.payload,
                    signature: fragment.signature,
                },
            )));
        }
        tracing::debug!(
            target: "rekindle_video::send",
            frame_seq = self.frame_seq,
            fragment_count = data_count,
            total_bytes = data_bytes,
            stream_id = %hex::encode(self.stream_id),
            keyframe = self.keyframe,
            "built video frame"
        );
        for fragment in fec.parity {
            envelopes.push(CommunityEnvelope::Control(
                ControlPayload::VideoParityFragment(VideoParityFragmentPayload {
                    channel_id: self.channel_id.to_string(),
                    stream_id: fragment.stream_id,
                    frame_seq: fragment.frame_seq,
                    parity_index: fragment.parity_index,
                    parity_total: fragment.parity_total,
                    data_count: fragment.data_count,
                    codec: fragment.codec,
                    frame_len: fragment.frame_len,
                    timestamp: fragment.timestamp,
                    key_index: fragment.key_index,
                    payload: fragment.payload,
                    signature: fragment.signature,
                }),
            ));
        }
        tracing::debug!(
            target: "rekindle_video::send::parity",
            frame_seq = self.frame_seq,
            parity_count = parity_count_total,
            total_bytes = parity_bytes,
            stream_id = %hex::encode(self.stream_id),
            data_shards = data_count,
            "built parity fragments"
        );
        Ok(envelopes)
    }
}

#[cfg(test)]
#[path = "send/tests.rs"]
mod tests;

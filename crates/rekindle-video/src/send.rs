//! Phase 16 — community video send pipeline (build side).
//!
//! Architecture §10.6 — MEK-encrypt the encoded payload, fragment to
//! the 4 KiB transport budget, sign each fragment with the community
//! pseudonym Ed25519
//! key. Phase 4: the signed envelopes are NOT dispatched here — they
//! are returned as one `PacedFrame` and released through the
//! audio-first `VideoPacer` (`send_pacer::run_video_pacer`), which
//! fans out to the channel roster via `VideoDeps::send_to_channel` —
//! never to the community gossip mesh.
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
use crate::pacer::PacedFrame;
use crate::reassembly_state::VideoReassemblyState;

/// One parity per N data shards for any multi-fragment frame. With 4×
/// ratio, dropping up to 25% of fragments is recoverable. The old
/// keyframes-only gate predates the 4 KiB fragment budget: a lost
/// delta fragment now costs a keyframe request (a full intra on the
/// wire), which is far more expensive than 25% parity on the delta.
const PARITY_RATIO_DENOM: usize = 4;

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
/// envelopes as ONE `PacedFrame` for the pacer to release at the
/// budgeted rate.
pub fn build_video_frame<D: VideoDeps>(
    deps: &D,
    reassembly: &VideoReassemblyState,
    community_id: &str,
    channel_id: &str,
    request: &VideoFrameSend,
    now_ms: u64,
) -> Result<PacedFrame, VideoError> {
    // Phase F — IPC entry trace. The Tauri command in src-tauri delivered
    // an encoded VP9 chunk from the WebView; record the byte count and
    // routing context (no payload bytes) so a `RUST_LOG=rekindle_video=
    // debug` operator can confirm the chunk arrived from the frontend
    // before MEK-encrypt + fragment + sign run.
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

    let provider = deps.keys();
    let unavailable = || VideoError::MekUnavailable {
        community: community_id.to_string(),
    };
    let scope = rekindle_types::id::ChannelId::from_hex(channel_id)
        .map(|channel| provider.scope_for_media(community_id, channel))
        .ok_or_else(unavailable)?;
    let (epoch, secret) =
        rekindle_types::channel_keys::current_key(&*provider, community_id, scope)
            .ok_or_else(unavailable)?;
    let mek_gen = epoch.0;
    let mek = rekindle_crypto::group::media_key::MediaEncryptionKey::from_bytes(*secret, mek_gen);
    let ciphertext = mek
        .encrypt(&request.encoded_payload)
        .map_err(|e| VideoError::Encrypt(format!("MEK encrypt: {e}")))?;

    let signing_key = deps
        .community_signing_key(community_id)
        .ok_or(VideoError::IdentityNotLoaded)?;

    let ctx = SendCtx {
        channel_id,
        stream_id: request.stream_id,
        frame_seq: request.frame_seq,
        keyframe: request.keyframe,
        codec: request.codec,
        timestamp: request.timestamp,
        mek_generation: mek_gen,
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
    let bytes = ciphertext.len();
    Ok(PacedFrame {
        community_id: community_id.to_string(),
        channel_id: channel_id.to_string(),
        stream_id: request.stream_id,
        frame_seq: request.frame_seq,
        keyframe: request.keyframe,
        envelopes,
        bytes,
        enqueued_ms: now_ms,
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
    /// Generation of the channel-media MEK that encrypted the frame —
    /// rides every fragment so receivers can request the exact key.
    mek_generation: u64,
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
            mek_generation: self.mek_generation,
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
                    mek_generation: fragment.mek_generation,
                    // Placeholder — the pacer stamps the real gap-free
                    // transport sequence at egress (`VideoPacer::poll`).
                    transport_seq: 0,
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
                    mek_generation: fragment.mek_generation,
                    // Placeholder — stamped by `VideoPacer::poll` at egress.
                    transport_seq: 0,
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
                    mek_generation: fragment.mek_generation,
                    // Placeholder — stamped by `VideoPacer::poll` at egress.
                    transport_seq: 0,
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

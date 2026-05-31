//! Phase 16 — community video send pipeline.
//!
//! Architecture §10.6 — MEK-encrypt the VP9 payload, fragment to ≤28 KB,
//! sign each fragment with the community pseudonym Ed25519 key, then
//! dispatch fragments + (for keyframes) FEC parity to the community
//! mesh via `VideoDeps::send_to_mesh`.
//!
//! The reassembly state is consulted ONLY to fire a one-shot
//! `TopologyChange { reason: "initial" }` per (community, stream) so
//! receivers know to spin up a decoder.

use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};

use crate::deps::VideoDeps;
use crate::error::VideoError;
use crate::fragment::{
    fragment_frame, fragment_frame_with_fec, fragment_signing_bytes, parity_signing_bytes,
    FRAGMENT_PAYLOAD_LIMIT,
};
use crate::reassembly_state::VideoReassemblyState;

/// One parity per N data shards for keyframes. With 4× ratio,
/// dropping up to 25% of fragments is recoverable. Inter-frames get
/// no parity.
const KEYFRAME_PARITY_RATIO_DENOM: usize = 4;

/// Per-frame send request. Bundling all the variable-per-frame fields
/// into a struct keeps the orchestration helpers below a sane argument
/// count and gives the Tauri command a clear input shape.
#[derive(Debug)]
pub struct VideoFrameSend {
    pub stream_id: [u8; 16],
    pub frame_seq: u32,
    pub keyframe: bool,
    pub timestamp: u32,
    pub encoded_payload: Vec<u8>,
}

/// Send-side entry point — invoked from the `send_video_frame` Tauri
/// command after the webview encoder produces a `VideoEncoder.encode()`
/// chunk. MEK-encrypts the payload, fragments to ≤28 KB, signs each
/// fragment with the sender's pseudonym Ed25519 key, and dispatches
/// each fragment as a `ControlPayload::VideoFragment` to the community
/// mesh.
pub fn send_video_frame<D: VideoDeps>(
    deps: &D,
    reassembly: &VideoReassemblyState,
    community_id: &str,
    channel_id: &str,
    request: &VideoFrameSend,
) -> Result<u32, VideoError> {
    if request.encoded_payload.is_empty() {
        return Err(VideoError::InvalidInput("empty encoded payload".into()));
    }

    let (mek_bytes, mek_gen) = deps.community_mek_bytes(community_id).ok_or_else(|| {
        VideoError::MekUnavailable {
            community: community_id.to_string(),
        }
    })?;
    let mek = rekindle_crypto::group::media_key::MediaEncryptionKey::from_bytes(mek_bytes, mek_gen);
    let ciphertext = mek
        .encrypt(&request.encoded_payload)
        .map_err(|e| VideoError::Encrypt(format!("MEK encrypt: {e}")))?;

    let signing_key = deps
        .community_signing_key(community_id)
        .ok_or(VideoError::IdentityNotLoaded)?;

    let ctx = SendCtx {
        deps,
        community_id,
        channel_id,
        stream_id: request.stream_id,
        frame_seq: request.frame_seq,
        keyframe: request.keyframe,
        timestamp: request.timestamp,
        signing_key: &signing_key,
    };

    // Architecture §32 Phase 6 Week 22 — emit a `TopologyChange { reason
    // = "initial" }` exactly once per stream so receivers know to spin
    // up a decoder. `mark_stream_started` returns true exactly once per
    // (community, stream) since startup.
    if reassembly.mark_stream_started(community_id, request.stream_id) {
        emit_initial_topology(deps, community_id, channel_id, request.stream_id)?;
    }

    let parity_count = parity_count_for(request.keyframe, &ciphertext);
    if parity_count > 0 {
        ctx.send_with_fec(&ciphertext, parity_count)
    } else {
        ctx.send_without_fec(&ciphertext)
    }
}

fn emit_initial_topology<D: VideoDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    stream_id: [u8; 16],
) -> Result<(), VideoError> {
    let lamport = deps.increment_lamport(community_id);
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
    deps.send_to_mesh(community_id, &envelope)
}

#[must_use]
fn parity_count_for(keyframe: bool, ciphertext: &[u8]) -> u8 {
    if !keyframe {
        return 0;
    }
    let data = ciphertext.len().div_ceil(FRAGMENT_PAYLOAD_LIMIT);
    if data < 2 {
        // 1-shard frames don't benefit from parity (parity = duplicate)
        // — and reed-solomon over 1+1 only recovers exact duplicates.
        return 0;
    }
    u8::try_from(data.div_ceil(KEYFRAME_PARITY_RATIO_DENOM)).unwrap_or(u8::MAX)
}

/// Bundle of references the FEC and non-FEC dispatch helpers both
/// need. Keeps each helper at one parameter (`ciphertext`) plus the
/// shared context.
struct SendCtx<'a, D: VideoDeps> {
    deps: &'a D,
    community_id: &'a str,
    channel_id: &'a str,
    stream_id: [u8; 16],
    frame_seq: u32,
    keyframe: bool,
    timestamp: u32,
    signing_key: &'a SigningKey,
}

impl<D: VideoDeps> SendCtx<'_, D> {
    fn send_without_fec(&self, ciphertext: &[u8]) -> Result<u32, VideoError> {
        let mut fragments = fragment_frame(
            self.stream_id,
            self.frame_seq,
            self.keyframe,
            self.timestamp,
            ciphertext,
        )?;
        let count = u32::try_from(fragments.len()).unwrap_or(u32::MAX);
        for fragment in &mut fragments {
            let to_sign = fragment_signing_bytes(fragment);
            fragment.signature = self.signing_key.sign(&to_sign).to_bytes().to_vec();
        }
        for fragment in fragments {
            let envelope = CommunityEnvelope::Control(ControlPayload::VideoFragment {
                channel_id: self.channel_id.to_string(),
                stream_id: fragment.stream_id,
                frame_seq: fragment.frame_seq,
                frag_index: fragment.frag_index,
                frag_total: fragment.frag_total,
                keyframe: fragment.keyframe,
                timestamp: fragment.timestamp,
                payload: fragment.payload,
                signature: fragment.signature,
            });
            self.deps.send_to_mesh(self.community_id, &envelope)?;
        }
        Ok(count)
    }

    fn send_with_fec(&self, ciphertext: &[u8], parity_count: u8) -> Result<u32, VideoError> {
        let mut fec = fragment_frame_with_fec(
            self.stream_id,
            self.frame_seq,
            self.keyframe,
            self.timestamp,
            ciphertext,
            parity_count,
        )?;

        for fragment in &mut fec.data {
            let to_sign = fragment_signing_bytes(fragment);
            fragment.signature = self.signing_key.sign(&to_sign).to_bytes().to_vec();
        }
        for fragment in &mut fec.parity {
            let to_sign = parity_signing_bytes(fragment);
            fragment.signature = self.signing_key.sign(&to_sign).to_bytes().to_vec();
        }

        let total = u32::try_from(fec.data.len() + fec.parity.len()).unwrap_or(u32::MAX);
        for fragment in fec.data {
            let envelope = CommunityEnvelope::Control(ControlPayload::VideoFragment {
                channel_id: self.channel_id.to_string(),
                stream_id: fragment.stream_id,
                frame_seq: fragment.frame_seq,
                frag_index: fragment.frag_index,
                frag_total: fragment.frag_total,
                keyframe: fragment.keyframe,
                timestamp: fragment.timestamp,
                payload: fragment.payload,
                signature: fragment.signature,
            });
            self.deps.send_to_mesh(self.community_id, &envelope)?;
        }
        for fragment in fec.parity {
            let envelope = CommunityEnvelope::Control(ControlPayload::VideoParityFragment {
                channel_id: self.channel_id.to_string(),
                stream_id: fragment.stream_id,
                frame_seq: fragment.frame_seq,
                parity_index: fragment.parity_index,
                parity_total: fragment.parity_total,
                data_count: fragment.data_count,
                frame_len: fragment.frame_len,
                timestamp: fragment.timestamp,
                payload: fragment.payload,
                signature: fragment.signature,
            });
            self.deps.send_to_mesh(self.community_id, &envelope)?;
        }
        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reassembly_state::VideoReassemblyState;
    use crate::test_mock::MockDeps;
    use rekindle_protocol::dht::community::envelope::ControlPayload;

    fn small_request(keyframe: bool) -> VideoFrameSend {
        VideoFrameSend {
            stream_id: [9u8; 16],
            frame_seq: 1,
            keyframe,
            timestamp: 100,
            encoded_payload: vec![0xAB; 256],
        }
    }

    #[test]
    fn empty_payload_rejected() {
        let deps = MockDeps::new();
        let reassembly = VideoReassemblyState::new();
        let mut req = small_request(false);
        req.encoded_payload.clear();
        let err = send_video_frame(&deps, &reassembly, "c1", "ch1", &req).unwrap_err();
        assert!(matches!(err, VideoError::InvalidInput(_)));
    }

    #[test]
    fn missing_mek_rejected() {
        let deps = MockDeps::without_mek();
        let reassembly = VideoReassemblyState::new();
        let err = send_video_frame(&deps, &reassembly, "c1", "ch1", &small_request(false))
            .unwrap_err();
        assert!(matches!(err, VideoError::MekUnavailable { .. }));
    }

    #[test]
    fn missing_identity_rejected() {
        let deps = MockDeps::without_signing_key();
        let reassembly = VideoReassemblyState::new();
        let err = send_video_frame(&deps, &reassembly, "c1", "ch1", &small_request(false))
            .unwrap_err();
        assert!(matches!(err, VideoError::IdentityNotLoaded));
    }

    #[test]
    fn first_frame_emits_initial_topology_change() {
        let deps = MockDeps::new();
        let reassembly = VideoReassemblyState::new();
        let count = send_video_frame(&deps, &reassembly, "c1", "ch1", &small_request(false))
            .expect("send happy path");
        assert!(count >= 1, "at least one fragment");
        let calls = deps.calls.lock();
        // First sent envelope should be the TopologyChange { reason: "initial" }.
        let first = calls.sent.first().expect("at least one envelope sent");
        assert!(matches!(
            first,
            CommunityEnvelope::Control(ControlPayload::TopologyChange { reason, .. }) if reason == "initial"
        ));
        // Subsequent envelopes are VideoFragment.
        for env in calls.sent.iter().skip(1) {
            assert!(matches!(
                env,
                CommunityEnvelope::Control(ControlPayload::VideoFragment { .. })
            ));
        }
    }

    #[test]
    fn second_frame_same_stream_skips_initial_topology() {
        let deps = MockDeps::new();
        let reassembly = VideoReassemblyState::new();
        send_video_frame(&deps, &reassembly, "c1", "ch1", &small_request(false)).unwrap();
        let envelopes_after_first = deps.calls.lock().sent.len();
        // Send another inter-frame on the same stream.
        let mut req2 = small_request(false);
        req2.frame_seq = 2;
        send_video_frame(&deps, &reassembly, "c1", "ch1", &req2).unwrap();
        let envelopes_after_second = deps.calls.lock().sent.len();
        let second_batch = envelopes_after_second - envelopes_after_first;
        // The second send should NOT include another TopologyChange.
        let calls = deps.calls.lock();
        let topology_count = calls
            .sent
            .iter()
            .filter(|e| matches!(e, CommunityEnvelope::Control(ControlPayload::TopologyChange { .. })))
            .count();
        assert_eq!(topology_count, 1, "TopologyChange fires exactly once per stream");
        assert!(second_batch >= 1, "second send produces at least 1 fragment");
    }

    #[test]
    fn parity_count_for_inter_frame_is_zero() {
        let ct = vec![0u8; 10_000];
        assert_eq!(parity_count_for(false, &ct), 0);
    }

    #[test]
    fn parity_count_for_single_shard_keyframe_is_zero() {
        let ct = vec![0u8; 100]; // < FRAGMENT_PAYLOAD_LIMIT
        assert_eq!(parity_count_for(true, &ct), 0);
    }

    #[test]
    fn parity_count_for_multi_shard_keyframe_is_positive() {
        let ct = vec![0u8; FRAGMENT_PAYLOAD_LIMIT * 4 + 100];
        let p = parity_count_for(true, &ct);
        assert!(p >= 1, "expected at least 1 parity shard for 5-shard keyframe");
    }
}

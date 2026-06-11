//! Phase 16 — community video send pipeline (build side).
//!
//! Architecture §10.6 — MEK-encrypt the encoded payload, fragment to
//! ≤28 KB, sign each fragment with the community pseudonym Ed25519
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

use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
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
    /// Codec the frontend encoder produced this chunk with — travels
    /// on every fragment (RTP payload-type analog) and is covered by
    /// the fragment signature.
    pub codec: Codec,
    pub timestamp: u32,
    pub encoded_payload: Vec<u8>,
}

/// Build-side entry point — invoked from the `send_video_frame` Tauri
/// command after the webview encoder produces a `VideoEncoder.encode()`
/// chunk. MEK-encrypts the payload, fragments to ≤28 KB, signs each
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

    let (mek_bytes, mek_gen) = deps
        .channel_media_mek(community_id, channel_id)
        .ok_or_else(|| VideoError::MekUnavailable {
            community: community_id.to_string(),
        })?;
    let mek = rekindle_crypto::group::media_key::MediaEncryptionKey::from_bytes(mek_bytes, mek_gen);
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

    let parity_count = parity_count_for(request.keyframe, &ciphertext);
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
    deps.send_to_channel(community_id, channel_id, &envelope)
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
                CommunityEnvelope::Control(ControlPayload::VideoFragment {
                    channel_id: self.channel_id.to_string(),
                    stream_id: fragment.stream_id,
                    frame_seq: fragment.frame_seq,
                    frag_index: fragment.frag_index,
                    frag_total: fragment.frag_total,
                    keyframe: fragment.keyframe,
                    codec: fragment.codec,
                    timestamp: fragment.timestamp,
                    mek_generation: fragment.mek_generation,
                    payload: fragment.payload,
                    signature: fragment.signature,
                })
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
            envelopes.push(CommunityEnvelope::Control(ControlPayload::VideoFragment {
                channel_id: self.channel_id.to_string(),
                stream_id: fragment.stream_id,
                frame_seq: fragment.frame_seq,
                frag_index: fragment.frag_index,
                frag_total: fragment.frag_total,
                keyframe: fragment.keyframe,
                codec: fragment.codec,
                timestamp: fragment.timestamp,
                mek_generation: fragment.mek_generation,
                payload: fragment.payload,
                signature: fragment.signature,
            }));
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
                ControlPayload::VideoParityFragment {
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
                    payload: fragment.payload,
                    signature: fragment.signature,
                },
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
            codec: Codec::Vp9,
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
        let err = build_video_frame(&deps, &reassembly, "c1", "ch1", &req, 0).unwrap_err();
        assert!(matches!(err, VideoError::InvalidInput(_)));
    }

    #[test]
    fn missing_mek_rejected() {
        let deps = MockDeps::without_mek();
        let reassembly = VideoReassemblyState::new();
        let err = build_video_frame(&deps, &reassembly, "c1", "ch1", &small_request(false), 0)
            .unwrap_err();
        assert!(matches!(err, VideoError::MekUnavailable { .. }));
    }

    #[test]
    fn missing_identity_rejected() {
        let deps = MockDeps::without_signing_key();
        let reassembly = VideoReassemblyState::new();
        let err = build_video_frame(&deps, &reassembly, "c1", "ch1", &small_request(false), 0)
            .unwrap_err();
        assert!(matches!(err, VideoError::IdentityNotLoaded));
    }

    /// Phase F smoke test — install a process-wide `tracing` Layer that
    /// records every event's target + field set, then exercise
    /// `send_video_frame`. Confirms the `rekindle_video::send` target
    /// with structured fields actually fires (which is what
    /// `RUST_LOG=rekindle_video=debug` operators see at runtime).
    ///
    /// **Why global, not per-thread.** `tracing` caches per-callsite
    /// `Interest` decisions on first registration. If parallel sibling
    /// tests reach the same callsite first with no subscriber, the
    /// macro is cached as "never enabled" for the entire process and
    /// later per-thread `with_default` subscribers can't observe the
    /// event. Setting one global Layer up-front via `Once` makes the
    /// callsites permanently enabled for the test binary.
    #[test]
    fn structured_trace_emits_on_send() {
        use std::sync::Arc;

        use parking_lot::Mutex;
        use tracing::field::{Field, Visit};
        use tracing_subscriber::layer::{Context, SubscriberExt};
        use tracing_subscriber::registry::Registry;
        use tracing_subscriber::util::SubscriberInitExt;
        use tracing_subscriber::Layer;

        #[derive(Debug, Default)]
        struct Captured {
            target: String,
            message: String,
            fields: Vec<String>,
        }

        #[derive(Clone, Default)]
        struct Sink(Arc<Mutex<Vec<Captured>>>);

        impl<S: tracing::Subscriber> Layer<S> for Sink {
            fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
                let mut rec = Captured {
                    target: event.metadata().target().to_string(),
                    ..Captured::default()
                };
                struct V<'a>(&'a mut Captured);
                impl Visit for V<'_> {
                    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
                        let formatted = format!("{value:?}");
                        if field.name() == "message" {
                            self.0.message = formatted;
                        } else {
                            self.0.fields.push(field.name().to_string());
                        }
                    }
                }
                event.record(&mut V(&mut rec));
                self.0.lock().push(rec);
            }
        }

        // One global sink shared by every test; the smoke test reads
        // its own slice by filtering on the unique community_id we use
        // for `send_video_frame` (sentinel: `"c_smoke_phase_f"`).
        //
        // We also chain an stderr-fmt layer so the Phase F smoke
        // verification shell command
        //   RUST_LOG=… cargo test -p rekindle-video --lib send 2>&1 \
        //     | grep "rekindle_video::send"
        // observes the same structured output the assertions below
        // verify. The fmt layer respects `RUST_LOG`; the sink layer
        // does not — assertions stay deterministic regardless of env.
        static SINK: std::sync::OnceLock<Sink> = std::sync::OnceLock::new();
        let sink = SINK
            .get_or_init(|| {
                use tracing_subscriber::EnvFilter;
                let s = Sink::default();
                let fmt_layer = tracing_subscriber::fmt::layer()
                    .with_writer(std::io::stderr)
                    .with_filter(
                        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("off")),
                    );
                let _ = Registry::default()
                    .with(s.clone())
                    .with(fmt_layer)
                    .try_init();
                s
            })
            .clone();

        let snapshot_before = sink.0.lock().len();
        let deps = MockDeps::new();
        let reassembly = VideoReassemblyState::new();
        let mut req = small_request(false);
        // Distinct stream_id so our IPC-entry trace is identifiable
        // even if a parallel test fires the same callsite.
        req.stream_id = [0xF0; 16];
        build_video_frame(&deps, &reassembly, "c_smoke_phase_f", "ch_smoke", &req, 0)
            .expect("build happy path");

        let captured = sink.0.lock();
        let new_events: Vec<&Captured> = captured
            .iter()
            .skip(snapshot_before)
            .filter(|c| c.target == "rekindle_video::send")
            .collect();
        assert!(
            new_events.len() >= 2,
            "expected at least two `rekindle_video::send` events (IPC entry + post-send), got {}: {:?}",
            new_events.len(),
            new_events
                .iter()
                .map(|c| (&c.message, &c.fields))
                .collect::<Vec<_>>()
        );
        let has_frame_seq = new_events
            .iter()
            .any(|c| c.fields.iter().any(|f| f == "frame_seq"));
        let has_stream_id = new_events
            .iter()
            .any(|c| c.fields.iter().any(|f| f == "stream_id"));
        let has_fragment_count = new_events
            .iter()
            .any(|c| c.fields.iter().any(|f| f == "fragment_count"));
        let has_encoded_bytes = new_events
            .iter()
            .any(|c| c.fields.iter().any(|f| f == "encoded_bytes"));
        assert!(
            has_frame_seq && has_stream_id,
            "send events must carry frame_seq and stream_id; got: {new_events:?}"
        );
        assert!(
            has_fragment_count,
            "at least one send event must carry fragment_count; got: {new_events:?}"
        );
        assert!(
            has_encoded_bytes,
            "IPC-entry event must carry encoded_bytes; got: {new_events:?}"
        );
    }

    #[test]
    fn first_frame_emits_initial_topology_change() {
        let deps = MockDeps::new();
        let reassembly = VideoReassemblyState::new();
        let frame = build_video_frame(&deps, &reassembly, "c1", "ch1", &small_request(false), 7)
            .expect("build happy path");
        assert!(!frame.envelopes.is_empty(), "at least one fragment");
        assert_eq!(frame.enqueued_ms, 7);
        // The pacer-bypassing control envelope: exactly the initial
        // TopologyChange went straight to deps; the fragments did NOT.
        let calls = deps.calls.lock();
        assert_eq!(
            calls.sent.len(),
            1,
            "only the topology envelope dispatches here"
        );
        assert!(matches!(
            calls.sent.first().expect("topology sent"),
            CommunityEnvelope::Control(ControlPayload::TopologyChange { reason, .. }) if reason == "initial"
        ));
        // The returned frame carries the fragments for the pacer.
        for env in &frame.envelopes {
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
        build_video_frame(&deps, &reassembly, "c1", "ch1", &small_request(false), 0).unwrap();
        let mut req2 = small_request(false);
        req2.frame_seq = 2;
        let frame2 = build_video_frame(&deps, &reassembly, "c1", "ch1", &req2, 0).unwrap();
        // The second build must NOT dispatch another TopologyChange.
        let calls = deps.calls.lock();
        let topology_count = calls
            .sent
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    CommunityEnvelope::Control(ControlPayload::TopologyChange { .. })
                )
            })
            .count();
        assert_eq!(
            topology_count, 1,
            "TopologyChange fires exactly once per stream"
        );
        assert!(
            !frame2.envelopes.is_empty(),
            "second build produces at least 1 fragment"
        );
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
        assert!(
            p >= 1,
            "expected at least 1 parity shard for 5-shard keyframe"
        );
    }
}

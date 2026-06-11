//! Phase 16 — community video receive dispatcher.
//!
//! Routes inbound `ControlPayload::Video*` variants from the directed
//! channel-peer transport: fragment + parity fragment ingest into the
//! per-community reassembler, then MEK-decrypt the assembled frame and
//! emit `VideoEvent::FrameReady`. The other control variants (FrameAck,
//! KeyframeRequest, BandwidthEstimate, TopologyChange,
//! MediaCapabilities) map 1:1 to their VideoEvent variants.
//!
//! Architecture §10.6 reader-validates gate: every payload carries the
//! channel it belongs to, and anything addressed to a channel the
//! local user is not actively joined to is dropped up front — before
//! reassembly, MEK decrypt, or any event reaches the frontend. A
//! compliant sender only ever addresses the channel roster, so this
//! gate is defense-in-depth against non-compliant or stale senders.

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_protocol::dht::community::envelope::ControlPayload;

use crate::deps::{VideoDeps, VideoEvent};
use crate::reassembler::ReassembledFrame;
use crate::reassembly_state::VideoReassemblyState;
use crate::{VideoFragment, VideoParityFragment};

/// The channel a video-flavoured `ControlPayload` is addressed to.
/// `None` for every non-video variant. Shared with the src-tauri
/// ingress (relay exclusion, rejected-toast gating, rate-floor
/// exemption) so "is video + which channel" has one definition.
#[must_use]
pub fn video_payload_channel(payload: &ControlPayload) -> Option<&str> {
    match payload {
        ControlPayload::VideoFragment { channel_id, .. }
        | ControlPayload::VideoParityFragment { channel_id, .. }
        | ControlPayload::FrameAck { channel_id, .. }
        | ControlPayload::KeyframeRequest { channel_id, .. }
        | ControlPayload::BandwidthEstimate { channel_id, .. }
        | ControlPayload::TopologyChange { channel_id, .. }
        | ControlPayload::MediaCapabilities { channel_id, .. } => Some(channel_id),
        _ => None,
    }
}

/// Dispatch entry point — routed from the src-tauri Veilid control
/// receiver when any video-flavoured `ControlPayload` arrives. Each
/// variant either ingests fragments (which can produce a reassembled
/// frame → emit FrameReady) or passes directly to a VideoEvent.
pub fn handle_video_payload<D: VideoDeps>(
    deps: &D,
    reassembly: &VideoReassemblyState,
    community_id: &str,
    sender_pseudonym: &str,
    payload: ControlPayload,
    now_ms: u32,
) {
    let Some(payload_channel) = video_payload_channel(&payload) else {
        return;
    };
    let payload_channel = payload_channel.to_string();
    let active = deps.local_active_channel(community_id);
    if active.as_deref() != Some(payload_channel.as_str()) {
        tracing::debug!(
            target: "rekindle_video::receive",
            community_id = %community_id,
            sender_pseudonym = %sender_pseudonym,
            payload_channel = %payload_channel,
            active_channel = active.as_deref().unwrap_or("<none>"),
            "video payload for a channel we're not in — dropping"
        );
        return;
    }
    match payload {
        ControlPayload::VideoFragment {
            channel_id: _,
            stream_id,
            frame_seq,
            frag_index,
            frag_total,
            keyframe,
            codec,
            timestamp,
            mek_generation,
            payload,
            signature,
        } => {
            let payload_len = payload.len();
            tracing::debug!(
                target: "rekindle_video::receive",
                frame_seq = frame_seq,
                frag_index = frag_index,
                frag_total = frag_total,
                stream_id = %hex::encode(stream_id),
                keyframe = keyframe,
                payload_bytes = payload_len,
                "ingested video fragment"
            );
            let frag = VideoFragment {
                stream_id,
                frame_seq,
                frag_index,
                frag_total,
                keyframe,
                codec,
                timestamp,
                mek_generation,
                payload,
                signature,
            };
            // W26 — fragments are Ed25519-signed by the sender's
            // pseudonym; verify before reassembly. The MEK only proves
            // "some member" produced the bytes — without this check any
            // member could splice frames into another member's stream.
            let to_sign = crate::fragment::fragment_signing_bytes(&frag);
            if !fragment_signature_valid(sender_pseudonym, &to_sign, &frag.signature) {
                tracing::warn!(
                    target: "rekindle_video::verify",
                    community_id = %community_id,
                    sender_pseudonym = %sender_pseudonym,
                    stream_id = %hex::encode(frag.stream_id),
                    frame_seq = frag.frame_seq,
                    frag_index = frag.frag_index,
                    "video fragment signature rejected — dropping"
                );
                return;
            }
            if let Some(frame) = reassembly.ingest(community_id, sender_pseudonym, frag, now_ms) {
                emit_frame_ready(
                    deps,
                    reassembly,
                    community_id,
                    &payload_channel,
                    sender_pseudonym,
                    &frame,
                    now_ms,
                );
            }
        }
        ControlPayload::VideoParityFragment {
            channel_id: _,
            stream_id,
            frame_seq,
            parity_index,
            parity_total,
            data_count,
            codec,
            frame_len,
            timestamp,
            mek_generation,
            payload,
            signature,
        } => {
            let payload_len = payload.len();
            tracing::debug!(
                target: "rekindle_video::receive::parity",
                frame_seq = frame_seq,
                parity_index = parity_index,
                parity_total = parity_total,
                data_count = data_count,
                stream_id = %hex::encode(stream_id),
                payload_bytes = payload_len,
                "ingested video parity fragment"
            );
            let frag = VideoParityFragment {
                stream_id,
                frame_seq,
                parity_index,
                parity_total,
                data_count,
                codec,
                frame_len,
                timestamp,
                mek_generation,
                payload,
                signature,
            };
            // W26 — same per-fragment authenticity gate as data
            // fragments; forged parity could otherwise corrupt
            // Reed-Solomon reconstruction of a legitimate frame.
            let to_sign = crate::fragment::parity_signing_bytes(&frag);
            if !fragment_signature_valid(sender_pseudonym, &to_sign, &frag.signature) {
                tracing::warn!(
                    target: "rekindle_video::verify",
                    community_id = %community_id,
                    sender_pseudonym = %sender_pseudonym,
                    stream_id = %hex::encode(frag.stream_id),
                    frame_seq = frag.frame_seq,
                    parity_index = frag.parity_index,
                    "video parity fragment signature rejected — dropping"
                );
                return;
            }
            if let Some(frame) =
                reassembly.ingest_parity(community_id, sender_pseudonym, frag, now_ms)
            {
                emit_frame_ready(
                    deps,
                    reassembly,
                    community_id,
                    &payload_channel,
                    sender_pseudonym,
                    &frame,
                    now_ms,
                );
            }
        }
        ControlPayload::FrameAck {
            channel_id,
            stream_id,
            last_frame_seq,
            kbps,
            loss_q8,
        } => {
            deps.emit_event(VideoEvent::FrameAck {
                community_id: community_id.to_string(),
                sender_pseudonym: sender_pseudonym.to_string(),
                channel_id,
                stream_id,
                last_frame_seq,
                kbps,
                loss_q8,
            });
        }
        ControlPayload::KeyframeRequest {
            channel_id,
            stream_id,
        } => {
            // Receiver dropped too many fragments; ask the frontend
            // (which owns the encoder) to mark the next frame as a
            // keyframe. Also reset our local reassembly buffer for
            // the same stream so we don't sit on stale partials.
            reassembly.reset_stream(community_id, stream_id, sender_pseudonym);
            deps.emit_event(VideoEvent::KeyframeRequest {
                community_id: community_id.to_string(),
                sender_pseudonym: sender_pseudonym.to_string(),
                channel_id,
                stream_id,
            });
        }
        ControlPayload::BandwidthEstimate {
            channel_id,
            kbps,
            window_secs,
            loss_q8,
        } => {
            deps.emit_event(VideoEvent::BandwidthEstimate {
                community_id: community_id.to_string(),
                sender_pseudonym: sender_pseudonym.to_string(),
                channel_id,
                kbps,
                window_secs,
                loss_q8,
            });
        }
        ControlPayload::TopologyChange {
            channel_id,
            stream_id,
            relay_host_pseudonym,
            reason,
            lamport,
        } => {
            // Architecture §10.6 + §22 — drop simultaneous topology
            // changes with stale lamport so the reassembler doesn't
            // flap between relays when two peers race a handover. The
            // higher lamport wins; equal lamport is also rejected
            // (the first observation already won).
            if !reassembly.accept_topology_change(community_id, stream_id, lamport) {
                tracing::debug!(
                    community = %community_id,
                    stream = %hex::encode(stream_id),
                    sender = %sender_pseudonym,
                    incoming_lamport = lamport,
                    "ignoring stale TopologyChange"
                );
                return;
            }
            // Discard any partial frames buffered against the previous
            // relay so the next frame from the new relay starts clean.
            reassembly.reset_stream(community_id, stream_id, sender_pseudonym);
            deps.emit_event(VideoEvent::TopologyChange {
                community_id: community_id.to_string(),
                sender_pseudonym: sender_pseudonym.to_string(),
                channel_id,
                stream_id,
                relay_host_pseudonym,
                reason,
                lamport,
            });
        }
        ControlPayload::MediaCapabilities {
            channel_id,
            max_pixel_count,
            max_fps,
            encode_codecs,
            decode_codecs,
            supports_optimize_for_latency,
            supported_scalability_modes,
        } => {
            deps.emit_event(VideoEvent::MediaCapabilities {
                community_id: community_id.to_string(),
                sender_pseudonym: sender_pseudonym.to_string(),
                channel_id,
                max_pixel_count,
                max_fps,
                encode_codecs,
                decode_codecs,
                supports_optimize_for_latency,
                supported_scalability_modes,
            });
        }
        _ => {}
    }
}

/// W26 — verify a fragment-level Ed25519 signature against the OUTER
/// envelope's sender pseudonym (already envelope-verified upstream).
/// Same boundary as the presence scan: the raw verify lives in
/// `rekindle-secrets`, the sole crypto importer.
fn fragment_signature_valid(sender_hex: &str, to_sign: &[u8], signature: &[u8]) -> bool {
    let Some(pseudonym) = hex::decode(sender_hex)
        .ok()
        .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
    else {
        return false;
    };
    let Ok(sig) = <[u8; 64]>::try_from(signature) else {
        return false;
    };
    rekindle_secrets::derive::verify_pseudonym_signature(&pseudonym, to_sign, &sig).is_ok()
}

/// Decrypt a reassembled frame under the channel-media MEK and emit
/// `VideoEvent::FrameReady` with the plaintext payload. The channel
/// gate in `handle_video_payload` already guarantees the frame belongs
/// to the channel we're actively in. Every fragment carries the
/// generation it was encrypted with, so recovery is CONVERGENT: a
/// miss, mismatch, or decrypt failure requests EXACTLY the frame's
/// generation (debounced) — never a guess. The same-generation
/// decrypt-failure case covers split-brain rotations (both sides
/// minted the same generation independently): the responder serves
/// its key for that generation and the requester overwrites.
fn emit_frame_ready<D: VideoDeps>(
    deps: &D,
    reassembly: &VideoReassemblyState,
    community_id: &str,
    channel_id: &str,
    sender_pseudonym: &str,
    frame: &ReassembledFrame,
    now_ms: u32,
) {
    let mut resolved = deps.channel_media_mek(community_id, channel_id);
    // A sender BEHIND our generation: during the rotation retention
    // window the REPLACED key still decrypts their in-flight frames
    // (SFrame RFC 9605 / DAVE previous-epoch retention) — no freeze on
    // every membership rotation. Past the window, drop without a
    // request (an older key can't help; apply refuses downgrades; the
    // sender converges from its own side).
    if let Some((_, our_gen)) = resolved {
        if frame.mek_generation < our_gen {
            match deps.previous_channel_mek(community_id, channel_id) {
                Some((prev_bytes, prev_gen)) if prev_gen == frame.mek_generation => {
                    resolved = Some((prev_bytes, prev_gen));
                }
                _ => {
                    tracing::debug!(
                        target: "rekindle_video::receive",
                        community_id = %community_id,
                        sender_pseudonym = %sender_pseudonym,
                        frame_generation = frame.mek_generation,
                        our_generation = our_gen,
                        "video frame from a sender behind our MEK generation — dropped"
                    );
                    return;
                }
            }
        }
    }
    let mismatch_reason = match resolved {
        None => Some("no channel-media MEK cached"),
        Some((_, our_gen)) if our_gen != frame.mek_generation => Some("MEK generation mismatch"),
        Some(_) => None,
    };
    if let Some(reason) = mismatch_reason {
        tracing::warn!(
            target: "rekindle_video::receive",
            community_id = %community_id,
            sender_pseudonym = %sender_pseudonym,
            frame_generation = frame.mek_generation,
            our_generation = resolved.map_or(-1i64, |(_, g)| i64::try_from(g).unwrap_or(i64::MAX)),
            reason,
            "video frame undecryptable — requesting the frame's exact MEK generation"
        );
        if reassembly.should_request_mek(community_id, now_ms) {
            deps.request_mek_refresh(community_id, channel_id, frame.mek_generation);
        }
        return;
    }
    let (mek_bytes, mek_gen) = resolved.expect("checked above");
    let mek = MediaEncryptionKey::from_bytes(mek_bytes, mek_gen);
    let plaintext = match mek.decrypt(&frame.payload) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(
                target: "rekindle_video::receive",
                error = %e,
                community_id = %community_id,
                sender_pseudonym = %sender_pseudonym,
                stream_id = %hex::encode(frame.stream_id),
                frame_seq = frame.frame_seq,
                frame_generation = frame.mek_generation,
                "video frame MEK decrypt failed at matching generation (split-brain or tamper) — requesting"
            );
            if reassembly.should_request_mek(community_id, now_ms) {
                deps.request_mek_refresh(community_id, channel_id, frame.mek_generation);
            }
            return;
        }
    };
    let plaintext_bytes = plaintext.len();
    tracing::info!(
        target: "rekindle_video::receive",
        frame_seq = frame.frame_seq,
        stream_id = %hex::encode(frame.stream_id),
        bytes = plaintext_bytes,
        keyframe = frame.keyframe,
        community_id = %community_id,
        sender_pseudonym = %sender_pseudonym,
        "frame reassembled"
    );
    deps.emit_event(VideoEvent::FrameReady {
        community_id: community_id.to_string(),
        sender_pseudonym: sender_pseudonym.to_string(),
        stream_id: frame.stream_id,
        frame_seq: frame.frame_seq,
        keyframe: frame.keyframe,
        codec: frame.codec,
        timestamp: frame.timestamp,
        payload: plaintext,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reassembly_state::VideoReassemblyState;
    use crate::test_mock::MockDeps;
    use rekindle_types::video::Codec;

    /// Build a single-fragment VideoFragment payload signed by the
    /// pseudonym derived from `seed` for community `"c1"`, MEK-encrypted
    /// under the MockDeps key. Returns `(sender_hex, payload)`.
    fn signed_fragment(seed: &[u8; 32], forge_signature: bool) -> (String, ControlPayload) {
        use rekindle_crypto::group::media_key::MediaEncryptionKey;
        use rekindle_secrets::derive::{derive_community_pseudonym, sign_with_pseudonym};

        let signing_key = derive_community_pseudonym(seed, "c1");
        let sender_hex = hex::encode(signing_key.verifying_key().to_bytes());
        let mek = MediaEncryptionKey::from_bytes([1u8; 32], 1);
        let ciphertext = mek.encrypt(b"vp9-keyframe-bytes").expect("mek encrypt");

        let mut frag = VideoFragment {
            stream_id: [3u8; 16],
            frame_seq: 1,
            frag_index: 0,
            frag_total: 1,
            keyframe: true,
            codec: Codec::Vp9,
            timestamp: 42,
            mek_generation: 1,
            payload: ciphertext,
            signature: Vec::new(),
        };
        frag.signature = if forge_signature {
            vec![0u8; 64]
        } else {
            sign_with_pseudonym(
                &signing_key,
                &crate::fragment::fragment_signing_bytes(&frag),
            )
            .to_vec()
        };
        (
            sender_hex,
            ControlPayload::VideoFragment {
                channel_id: "ch1".into(),
                stream_id: frag.stream_id,
                frame_seq: frag.frame_seq,
                frag_index: frag.frag_index,
                frag_total: frag.frag_total,
                keyframe: frag.keyframe,
                codec: frag.codec,
                timestamp: frag.timestamp,
                mek_generation: frag.mek_generation,
                payload: frag.payload,
                signature: frag.signature,
            },
        )
    }

    #[test]
    fn valid_fragment_signature_reaches_frame_ready() {
        let deps = MockDeps::new();
        let reassembly = VideoReassemblyState::new();
        let (sender_hex, payload) = signed_fragment(&[9u8; 32], false);
        handle_video_payload(&deps, &reassembly, "c1", &sender_hex, payload, 0);
        let calls = deps.calls.lock();
        assert!(
            calls
                .events
                .iter()
                .any(|e| matches!(e, VideoEvent::FrameReady { .. })),
            "signed + decryptable single-fragment frame must emit FrameReady"
        );
    }

    #[test]
    fn forged_fragment_signature_is_dropped_before_reassembly() {
        let deps = MockDeps::new();
        let reassembly = VideoReassemblyState::new();
        let (sender_hex, payload) = signed_fragment(&[9u8; 32], true);
        handle_video_payload(&deps, &reassembly, "c1", &sender_hex, payload, 0);
        let calls = deps.calls.lock();
        assert!(
            calls.events.is_empty(),
            "forged fragment signature must never produce an event"
        );
        assert!(
            calls.mek_refresh_requests.is_empty(),
            "forged fragments must not trigger MEK requests either"
        );
    }

    #[test]
    fn mismatched_sender_pseudonym_is_dropped() {
        // Valid signature from key A, but the envelope sender is B —
        // a member replaying another member's fragments under their
        // own envelope must be rejected.
        let deps = MockDeps::new();
        let reassembly = VideoReassemblyState::new();
        let (_real_sender, payload) = signed_fragment(&[9u8; 32], false);
        let other_sender = hex::encode([8u8; 32]);
        handle_video_payload(&deps, &reassembly, "c1", &other_sender, payload, 0);
        assert!(deps.calls.lock().events.is_empty());
    }

    #[test]
    fn mek_mismatch_fires_debounced_refresh_request() {
        use rekindle_crypto::group::media_key::MediaEncryptionKey;
        use rekindle_secrets::derive::{derive_community_pseudonym, sign_with_pseudonym};

        // Sender encrypts under generation 2; MockDeps holds gen 1
        // with DIFFERENT bytes → decrypt fails → RequestMEK fires once
        // (debounced) instead of a silent drop.
        let deps = MockDeps::new();
        let reassembly = VideoReassemblyState::new();

        let signing_key = derive_community_pseudonym(&[9u8; 32], "c1");
        let sender_hex = hex::encode(signing_key.verifying_key().to_bytes());
        let newer_mek = MediaEncryptionKey::from_bytes([2u8; 32], 2);

        let make_payload = |frame_seq: u32| {
            let mut frag = VideoFragment {
                stream_id: [4u8; 16],
                frame_seq,
                frag_index: 0,
                frag_total: 1,
                keyframe: true,
                codec: Codec::Vp9,
                timestamp: 7,
                mek_generation: 2,
                payload: newer_mek.encrypt(b"frame").expect("encrypt"),
                signature: Vec::new(),
            };
            frag.signature = sign_with_pseudonym(
                &signing_key,
                &crate::fragment::fragment_signing_bytes(&frag),
            )
            .to_vec();
            ControlPayload::VideoFragment {
                channel_id: "ch1".into(),
                stream_id: frag.stream_id,
                frame_seq: frag.frame_seq,
                frag_index: frag.frag_index,
                frag_total: frag.frag_total,
                keyframe: frag.keyframe,
                codec: frag.codec,
                timestamp: frag.timestamp,
                mek_generation: frag.mek_generation,
                payload: frag.payload,
                signature: frag.signature,
            }
        };

        handle_video_payload(&deps, &reassembly, "c1", &sender_hex, make_payload(1), 0);
        // Second failing frame inside the debounce window: no new request.
        handle_video_payload(
            &deps,
            &reassembly,
            "c1",
            &sender_hex,
            make_payload(2),
            1_000,
        );

        let calls = deps.calls.lock();
        assert_eq!(
            calls.mek_refresh_requests,
            vec![("c1".to_string(), "ch1".to_string())],
            "exactly one debounced MEK refresh request"
        );
        assert!(
            calls
                .events
                .iter()
                .all(|e| !matches!(e, VideoEvent::FrameReady { .. })),
            "undecryptable frames never reach the frontend"
        );
    }

    #[test]
    fn payload_for_other_channel_is_dropped() {
        // Local user is in ch1; a FrameAck addressed to ch2 must be
        // dropped before any event reaches the frontend.
        let deps = MockDeps::new();
        let reassembly = VideoReassemblyState::new();
        handle_video_payload(
            &deps,
            &reassembly,
            "c1",
            "peer1",
            ControlPayload::FrameAck {
                channel_id: "ch2".into(),
                stream_id: [5u8; 16],
                last_frame_seq: 7,
                kbps: 1000,
                loss_q8: 12,
            },
            0,
        );
        assert!(
            deps.calls.lock().events.is_empty(),
            "payload for a channel we're not in must not produce events"
        );
    }

    #[test]
    fn payload_with_no_active_session_is_dropped() {
        // Not in any voice/video channel: every video payload is
        // dropped, even with a community MEK cached.
        let deps = MockDeps::in_channel(None);
        let reassembly = VideoReassemblyState::new();
        handle_video_payload(
            &deps,
            &reassembly,
            "c1",
            "peer1",
            ControlPayload::MediaCapabilities {
                channel_id: "ch1".into(),
                max_pixel_count: 480 * 854,
                max_fps: 30,
                encode_codecs: vec![rekindle_types::video::Codec::Vp9],
                decode_codecs: vec![rekindle_types::video::Codec::Vp9],
                supports_optimize_for_latency: false,
                supported_scalability_modes: vec![rekindle_types::video::ScalabilityMode::Flat],
            },
            0,
        );
        assert!(
            deps.calls.lock().events.is_empty(),
            "no active session means no video consumption"
        );
    }

    #[test]
    fn fragment_for_other_channel_never_reaches_reassembly() {
        let deps = MockDeps::new();
        let reassembly = VideoReassemblyState::new();
        handle_video_payload(
            &deps,
            &reassembly,
            "c1",
            "peer1",
            ControlPayload::VideoFragment {
                channel_id: "ch2".into(),
                stream_id: [9u8; 16],
                frame_seq: 1,
                frag_index: 0,
                frag_total: 1,
                keyframe: false,
                codec: Codec::Vp9,
                timestamp: 0,
                mek_generation: 0,
                payload: vec![0xAB; 64],
                signature: vec![0u8; 64],
            },
            0,
        );
        assert!(
            deps.calls.lock().events.is_empty(),
            "single-fragment frame for another channel must not be reassembled or emitted"
        );
    }

    #[test]
    fn frame_ack_maps_to_event() {
        let deps = MockDeps::new();
        let reassembly = VideoReassemblyState::new();
        handle_video_payload(
            &deps,
            &reassembly,
            "c1",
            "peer1",
            ControlPayload::FrameAck {
                channel_id: "ch1".into(),
                stream_id: [5u8; 16],
                last_frame_seq: 7,
                kbps: 1000,
                loss_q8: 12,
            },
            0,
        );
        let calls = deps.calls.lock();
        assert_eq!(calls.events.len(), 1);
        let VideoEvent::FrameAck {
            last_frame_seq,
            kbps,
            ..
        } = &calls.events[0]
        else {
            panic!("expected FrameAck variant");
        };
        assert_eq!(*last_frame_seq, 7);
        assert_eq!(*kbps, 1000);
    }

    #[test]
    fn keyframe_request_resets_stream_and_emits_event() {
        let deps = MockDeps::new();
        let reassembly = VideoReassemblyState::new();
        handle_video_payload(
            &deps,
            &reassembly,
            "c1",
            "peer1",
            ControlPayload::KeyframeRequest {
                channel_id: "ch1".into(),
                stream_id: [3u8; 16],
            },
            0,
        );
        let calls = deps.calls.lock();
        assert!(matches!(
            calls.events[0],
            VideoEvent::KeyframeRequest { .. }
        ));
    }

    #[test]
    fn bandwidth_estimate_maps_to_event() {
        let deps = MockDeps::new();
        let reassembly = VideoReassemblyState::new();
        handle_video_payload(
            &deps,
            &reassembly,
            "c1",
            "peer1",
            ControlPayload::BandwidthEstimate {
                channel_id: "ch1".into(),
                kbps: 2500,
                window_secs: 5,
                loss_q8: 0,
            },
            0,
        );
        let calls = deps.calls.lock();
        let VideoEvent::BandwidthEstimate {
            kbps, window_secs, ..
        } = &calls.events[0]
        else {
            panic!("expected BandwidthEstimate variant");
        };
        assert_eq!(*kbps, 2500);
        assert_eq!(*window_secs, 5);
    }

    #[test]
    fn media_capabilities_maps_to_event() {
        let deps = MockDeps::new();
        let reassembly = VideoReassemblyState::new();
        handle_video_payload(
            &deps,
            &reassembly,
            "c1",
            "peer1",
            ControlPayload::MediaCapabilities {
                channel_id: "ch1".into(),
                max_pixel_count: 480 * 854,
                max_fps: 30,
                encode_codecs: vec![rekindle_types::video::Codec::H264],
                decode_codecs: vec![
                    rekindle_types::video::Codec::Vp9,
                    rekindle_types::video::Codec::H264,
                ],
                supports_optimize_for_latency: false,
                supported_scalability_modes: vec![rekindle_types::video::ScalabilityMode::Flat],
            },
            0,
        );
        let calls = deps.calls.lock();
        let VideoEvent::MediaCapabilities {
            ref encode_codecs,
            ref decode_codecs,
            ..
        } = calls.events[0]
        else {
            panic!("expected MediaCapabilities variant");
        };
        // Direction-split lists must map through without being swapped.
        assert_eq!(encode_codecs, &vec![rekindle_types::video::Codec::H264]);
        assert_eq!(
            decode_codecs,
            &vec![
                rekindle_types::video::Codec::Vp9,
                rekindle_types::video::Codec::H264,
            ]
        );
    }

    #[test]
    fn topology_change_higher_lamport_accepted() {
        let deps = MockDeps::new();
        let reassembly = VideoReassemblyState::new();
        handle_video_payload(
            &deps,
            &reassembly,
            "c1",
            "peer1",
            ControlPayload::TopologyChange {
                channel_id: "ch1".into(),
                stream_id: [1u8; 16],
                relay_host_pseudonym: None,
                reason: "switch".into(),
                lamport: 5,
            },
            0,
        );
        let calls = deps.calls.lock();
        assert_eq!(calls.events.len(), 1);
        assert!(matches!(calls.events[0], VideoEvent::TopologyChange { .. }));
    }

    #[test]
    fn topology_change_lower_lamport_rejected() {
        let deps = MockDeps::new();
        let reassembly = VideoReassemblyState::new();
        // Accept high lamport first.
        handle_video_payload(
            &deps,
            &reassembly,
            "c1",
            "peer1",
            ControlPayload::TopologyChange {
                channel_id: "ch1".into(),
                stream_id: [2u8; 16],
                relay_host_pseudonym: None,
                reason: "switch".into(),
                lamport: 10,
            },
            0,
        );
        let events_after_first = deps.calls.lock().events.len();
        // Lower lamport should be silently dropped.
        handle_video_payload(
            &deps,
            &reassembly,
            "c1",
            "peer1",
            ControlPayload::TopologyChange {
                channel_id: "ch1".into(),
                stream_id: [2u8; 16],
                relay_host_pseudonym: None,
                reason: "stale".into(),
                lamport: 5,
            },
            0,
        );
        let events_after_second = deps.calls.lock().events.len();
        assert_eq!(
            events_after_first, events_after_second,
            "stale lamport TopologyChange should be dropped"
        );
    }
}

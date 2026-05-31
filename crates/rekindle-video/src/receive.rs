//! Phase 16 — community video receive dispatcher.
//!
//! Routes inbound `ControlPayload::Video*` variants from the gossip
//! mesh: fragment + parity fragment ingest into the per-community
//! reassembler, then MEK-decrypt the assembled frame and emit
//! `VideoEvent::FrameReady`. The other 4 control variants (FrameAck,
//! KeyframeRequest, BandwidthEstimate, TopologyChange,
//! MediaCapabilities) map 1:1 to their VideoEvent variants.

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_protocol::dht::community::envelope::ControlPayload;

use crate::deps::{VideoDeps, VideoEvent};
use crate::reassembler::ReassembledFrame;
use crate::reassembly_state::VideoReassemblyState;
use crate::{VideoFragment, VideoParityFragment};

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
    match payload {
        ControlPayload::VideoFragment {
            channel_id: _,
            stream_id,
            frame_seq,
            frag_index,
            frag_total,
            keyframe,
            timestamp,
            payload,
            signature,
        } => {
            let frag = VideoFragment {
                stream_id,
                frame_seq,
                frag_index,
                frag_total,
                keyframe,
                timestamp,
                payload,
                signature,
            };
            if let Some(frame) = reassembly.ingest(community_id, sender_pseudonym, frag, now_ms) {
                emit_frame_ready(deps, community_id, sender_pseudonym, &frame);
            }
        }
        ControlPayload::VideoParityFragment {
            channel_id: _,
            stream_id,
            frame_seq,
            parity_index,
            parity_total,
            data_count,
            frame_len,
            timestamp,
            payload,
            signature,
        } => {
            let frag = VideoParityFragment {
                stream_id,
                frame_seq,
                parity_index,
                parity_total,
                data_count,
                frame_len,
                timestamp,
                payload,
                signature,
            };
            if let Some(frame) =
                reassembly.ingest_parity(community_id, sender_pseudonym, frag, now_ms)
            {
                emit_frame_ready(deps, community_id, sender_pseudonym, &frame);
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
            codecs,
        } => {
            deps.emit_event(VideoEvent::MediaCapabilities {
                community_id: community_id.to_string(),
                sender_pseudonym: sender_pseudonym.to_string(),
                channel_id,
                max_pixel_count,
                max_fps,
                codecs,
            });
        }
        _ => {}
    }
}

/// Decrypt a reassembled frame under the current community MEK and
/// emit `VideoEvent::FrameReady` with the plaintext payload. If no
/// MEK is cached (e.g. the user hasn't joined voice/video), drop the
/// frame silently — this matches pre-Phase-16 behavior.
fn emit_frame_ready<D: VideoDeps>(
    deps: &D,
    community_id: &str,
    sender_pseudonym: &str,
    frame: &ReassembledFrame,
) {
    let Some((mek_bytes, mek_gen)) = deps.community_mek_bytes(community_id) else {
        tracing::debug!(
            community = %community_id,
            "video frame received but no MEK cached — dropping"
        );
        return;
    };
    let mek = MediaEncryptionKey::from_bytes(mek_bytes, mek_gen);
    let plaintext = match mek.decrypt(&frame.payload) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "video frame MEK decrypt failed");
            return;
        }
    };
    deps.emit_event(VideoEvent::FrameReady {
        community_id: community_id.to_string(),
        sender_pseudonym: sender_pseudonym.to_string(),
        stream_id: frame.stream_id,
        frame_seq: frame.frame_seq,
        keyframe: frame.keyframe,
        timestamp: frame.timestamp,
        payload: plaintext,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reassembly_state::VideoReassemblyState;
    use crate::test_mock::MockDeps;

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
                codecs: vec!["vp9".into()],
            },
            0,
        );
        let calls = deps.calls.lock();
        assert!(matches!(
            calls.events[0],
            VideoEvent::MediaCapabilities { .. }
        ));
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

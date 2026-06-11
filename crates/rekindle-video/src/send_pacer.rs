//! Async driver for the video pacer — owns the release loop between
//! `build_video_frame` (producer: the Tauri send command) and
//! `VideoDeps::send_to_channel` (consumer: the channel-roster fan-out).
//!
//! One pacer task per voice session, spawned/torn down by the
//! src-tauri session setup. The rate input is the backend bitrate
//! policy's watch channel (`budget::target_from_feedback` output).

use std::sync::Arc;
use std::time::Duration;

use crate::deps::VideoDeps;
use crate::pacer::{PacedFrame, VideoPacer};

/// Cadence of the structured pacer summary log (Phase 7 anchor).
const SUMMARY_INTERVAL_MS: u64 = 5_000;

/// Run the pacer until `shutdown_rx` fires or the frame channel
/// closes. `now_ms` comes from `rekindle_utils::timestamp_ms` so the
/// pure pacer stays clock-free.
pub async fn run_video_pacer<D: VideoDeps>(
    deps: Arc<D>,
    mut frame_rx: tokio::sync::mpsc::Receiver<PacedFrame>,
    mut rate_rx: tokio::sync::watch::Receiver<u32>,
    mut shutdown_rx: tokio::sync::mpsc::Receiver<()>,
) {
    let mut pacer = VideoPacer::new(*rate_rx.borrow());
    let mut last_summary_ms = rekindle_utils::timestamp_ms();
    tracing::info!(
        target: "rekindle_video::pacer",
        rate_kbps = *rate_rx.borrow(),
        "video pacer started"
    );
    loop {
        let now = rekindle_utils::timestamp_ms();
        let wait = pacer.next_poll_in_ms(now);
        tokio::select! {
            biased;
            _ = shutdown_rx.recv() => {
                tracing::info!(target: "rekindle_video::pacer", "video pacer shutting down");
                break;
            }
            changed = rate_rx.changed() => {
                if changed.is_ok() {
                    let kbps = *rate_rx.borrow();
                    pacer.set_rate_kbps(kbps);
                    tracing::info!(
                        target: "rekindle_video::pacer",
                        rate_kbps = kbps,
                        "pacer rate updated"
                    );
                }
                // Sender dropped = session tearing down; the shutdown
                // arm or closed frame channel ends the loop.
            }
            frame = frame_rx.recv() => {
                let Some(frame) = frame else {
                    tracing::info!(
                        target: "rekindle_video::pacer",
                        "frame channel closed — pacer exiting"
                    );
                    break;
                };
                let now = rekindle_utils::timestamp_ms();
                let dropped = pacer.enqueue(frame, now);
                if dropped > 0 {
                    tracing::warn!(
                        target: "rekindle_video::pacer",
                        dropped,
                        queue_depth = pacer.stats().queue_depth,
                        "pacer saturated — shed stale frames"
                    );
                }
            }
            () = tokio::time::sleep(Duration::from_millis(wait.max(1))) => {}
        }

        let now = rekindle_utils::timestamp_ms();
        for (community_id, channel_id, envelope) in pacer.poll(now) {
            if let Err(error) = deps.send_to_channel(&community_id, &channel_id, &envelope) {
                tracing::warn!(
                    target: "rekindle_video::pacer",
                    %error,
                    community_id = %community_id,
                    channel_id = %channel_id,
                    "paced fragment send failed"
                );
            }
        }

        if now.saturating_sub(last_summary_ms) >= SUMMARY_INTERVAL_MS {
            last_summary_ms = now;
            let s = pacer.stats();
            tracing::info!(
                target: "rekindle_video::pacer",
                sent_fragments = s.sent_fragments,
                dropped_frames = s.dropped_frames,
                expired_frames = s.expired_frames,
                queue_depth = s.queue_depth,
                rate_kbps = s.rate_kbps,
                "pacer summary"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_mock::MockDeps;
    use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};

    fn paced_frame(seq: u32, fragments: usize) -> PacedFrame {
        let sid = u8::try_from(seq).unwrap_or(0);
        let envelopes = (0..fragments)
            .map(|i| {
                CommunityEnvelope::Control(ControlPayload::VideoFragment {
                    channel_id: "ch1".into(),
                    stream_id: [sid; 16],
                    frame_seq: seq,
                    frag_index: u8::try_from(i).unwrap(),
                    frag_total: u8::try_from(fragments).unwrap(),
                    keyframe: true,
                    codec: rekindle_types::video::Codec::Vp9,
                    timestamp: 0,
                    mek_generation: 0,
                    payload: vec![0; 4_000],
                    signature: Vec::new(),
                })
            })
            .collect();
        PacedFrame {
            community_id: "c1".into(),
            channel_id: "ch1".into(),
            stream_id: [sid; 16],
            frame_seq: seq,
            keyframe: true,
            envelopes,
            bytes: fragments * 4_000,
            enqueued_ms: rekindle_utils::timestamp_ms(),
        }
    }

    /// Frames in → every fragment eventually delivered, in order, at a
    /// constrained rate.
    #[tokio::test]
    async fn frames_drain_in_order_through_deps() {
        let deps = Arc::new(MockDeps::new());
        let (frame_tx, frame_rx) = tokio::sync::mpsc::channel(8);
        let (_rate_tx, rate_rx) = tokio::sync::watch::channel(350u32);
        let (shutdown_tx, shutdown_rx) = tokio::sync::mpsc::channel(1);

        let task = tokio::spawn(run_video_pacer(
            Arc::clone(&deps),
            frame_rx,
            rate_rx,
            shutdown_rx,
        ));

        frame_tx.send(paced_frame(1, 3)).await.unwrap();
        frame_tx.send(paced_frame(2, 2)).await.unwrap();

        // 5 fragments × 4 KB at 350 kbps ≈ 460 ms — allow generous slack.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            if deps.calls.lock().sent.len() >= 5 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "fragments must drain: got {}",
                deps.calls.lock().sent.len()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        {
            let calls = deps.calls.lock();
            let seqs: Vec<(u32, u8)> = calls
                .sent
                .iter()
                .filter_map(|e| match e {
                    CommunityEnvelope::Control(ControlPayload::VideoFragment {
                        frame_seq,
                        frag_index,
                        ..
                    }) => Some((*frame_seq, *frag_index)),
                    _ => None,
                })
                .collect();
            assert_eq!(
                seqs,
                vec![(1, 0), (1, 1), (1, 2), (2, 0), (2, 1)],
                "FIFO frame order + fragment order preserved"
            );
        }

        shutdown_tx.send(()).await.unwrap();
        task.await.unwrap();
    }
}

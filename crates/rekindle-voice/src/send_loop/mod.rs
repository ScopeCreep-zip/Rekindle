//! Phase 14 — voice capture → process → encode → transport send pipeline.
//!
//! Drains `capture_rx`, runs `AudioProcessor` (AEC + denoise + VAD),
//! encodes with Opus, optionally MEK-encrypts (community voice), and
//! sends via `VoiceTransport`. Owns the transport for the run.
//!
//! Parameterized over `Arc<dyn VoiceSessionDeps>` so the loop body
//! lives in the crate while AppState lookups (community MEK, stage
//! gate, etc.) flow through the deps trait. Pre-Phase-14 this lived in
//! `src-tauri/services/voice/send_loop.rs` (351 LoC).
//!
//! The src-tauri facade (lands in 14.i) builds the deps + params and
//! calls [`run`].

mod quality;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{broadcast, mpsc};

use crate::audio_processing::AudioProcessor;
use crate::codec::OpusCodec;
use crate::liveness::MediaLiveness;
use crate::media_crypto::{FrameSealer, MediaKeys, MediaScope};
use crate::receiver_report::VoiceReceiverReport;
use crate::send_loop::quality::PeerLink;
use crate::session_deps::{MediaKeySource, VoiceSessionDeps, VoiceSessionEvent};
use crate::transport::{OutboundFrame, VoiceTransport};
use crate::VoiceMode;

pub struct VoiceSendParams {
    pub capture_rx: Option<mpsc::Receiver<Vec<f32>>>,
    pub transport: Arc<tokio::sync::Mutex<VoiceTransport>>,
    /// Cancelled when the loop's session scope shuts down.
    pub stop: tokio_util::sync::CancellationToken,
    pub deps: Arc<dyn VoiceSessionDeps>,
    pub public_key: String,
    pub noise_suppression: bool,
    pub echo_cancellation: bool,
    pub muted_flag: Arc<AtomicBool>,
    pub speaker_ref_rx: broadcast::Receiver<Vec<f32>>,
    /// Community ID (with the channel, it names our sender key). `None` for
    /// 1:1 calls, whose channel id is the peer.
    pub community_id: Option<String>,
    /// Voice channel ID we're transmitting in. Used with the stage
    /// gate (§10.7) to drop frames from non-speakers.
    pub channel_id: String,
    /// Our pseudonym in this community (for stage-speaker check).
    /// `None` for 1:1 calls (no stage gate applies).
    pub our_pseudonym: Option<String>,
    /// Inbound RFC 3550 receiver reports, routed here by the dispatch
    /// loop. This is how the send loop learns what its audio looks like
    /// at the far end — loss, discard, jitter and round trip — none of
    /// which is derivable from a local `send()` result.
    pub report_rx: mpsc::Receiver<VoiceReceiverReport>,
    /// Session-shared media-plane liveness ledger. Every verified
    /// receiver report notes the reporter: a report proves the peer is
    /// alive even when VAD keeps them silent (no voice packets for the
    /// receive loop to note), so the presence reconcile can't evict a
    /// live-but-quiet peer whose DHT heartbeat writes are failing.
    pub media_liveness: Arc<MediaLiveness>,
}

struct VoiceSendLoop {
    capture_rx: mpsc::Receiver<Vec<f32>>,
    transport: Arc<tokio::sync::Mutex<VoiceTransport>>,
    stop: tokio_util::sync::CancellationToken,
    deps: Arc<dyn VoiceSessionDeps>,
    public_key: String,
    codec: OpusCodec,
    processor: AudioProcessor,
    muted_flag: Arc<AtomicBool>,
    speaker_ref_rx: broadcast::Receiver<Vec<f32>>,
    pcm_buffer: Vec<f32>,
    frame_size: usize,
    sequence: u32,
    was_speaking: bool,
    packets_sent: u64,
    /// Datagrams sent and failed over every route at the last quality
    /// pass, for the local-failure rate when no peer has reported.
    sends_at_last_pass: (u64, u64),
    /// The audio bitrate the allocator gave us (plan E4.3.3).
    audio_bps: u32,
    /// Diagnostic: frames seen by `process_frame`, for periodic capture
    /// level + VAD logging (the only outbound-voice observability we have).
    diag_frames: u64,
    last_quality_report: Instant,
    community_id: Option<String>,
    channel_id: String,
    our_pseudonym: Option<String>,
    report_rx: mpsc::Receiver<VoiceReceiverReport>,
    /// Per-peer view of our outbound stream, keyed by the reporter's
    /// pseudonym hex. Only peers we have actually heard from appear
    /// here: a peer that never reports must keep the local-failure
    /// classification, not be declared lost for staying quiet.
    peer_links: HashMap<String, PeerLink>,
    media_liveness: Arc<MediaLiveness>,
    /// The session's media routes, taken once at start: each receiver
    /// report's round trip goes to its route's estimator (plan E4.3 T2).
    media: Option<Arc<crate::transport::roster::MediaRoster>>,
    /// SFrame-seals every outbound frame under our sender key.
    sealer: FrameSealer,
}

/// Entry point: validate params, build loop state, run until shutdown.
pub async fn run(params: VoiceSendParams) {
    let Some(loop_state) = VoiceSendLoop::new(params) else {
        return;
    };
    loop_state.run_loop().await;
}

impl VoiceSendLoop {
    fn new(params: VoiceSendParams) -> Option<Self> {
        let Some(capture_rx) = params.capture_rx else {
            tracing::warn!("voice send loop started without capture_rx — exiting");
            return None;
        };

        let sample_rate: u32 = crate::SAMPLE_RATE_HZ;
        let channels: u16 = crate::CHANNELS;
        let frame_size: usize = crate::FRAME_SAMPLES_20MS;

        let codec = match OpusCodec::new(sample_rate, channels, frame_size) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(error = %e, "voice send loop: failed to create Opus codec");
                return None;
            }
        };

        let frame_duration_ms = u32::try_from(frame_size).unwrap_or(960) * 1000 / sample_rate;
        let processor = AudioProcessor::new(
            params.noise_suppression,
            params.echo_cancellation,
            0.02, // vad_threshold
            300,  // vad_hold_ms
            frame_duration_ms,
        );

        let key_source: Arc<dyn MediaKeySource> = params.deps.clone();
        let keys = Arc::new(MediaKeys::new(
            key_source,
            MediaScope::of_session(params.community_id.as_deref(), &params.channel_id),
        ));
        Some(Self {
            sealer: FrameSealer::new(keys),
            capture_rx,
            transport: params.transport,
            stop: params.stop,
            deps: params.deps,
            public_key: params.public_key,
            codec,
            processor,
            muted_flag: params.muted_flag,
            speaker_ref_rx: params.speaker_ref_rx,
            pcm_buffer: Vec::with_capacity(frame_size * 2),
            frame_size,
            sequence: 0,
            was_speaking: false,
            packets_sent: 0,
            sends_at_last_pass: (0, 0),
            audio_bps: crate::transport::allocation::Allocation::default().audio_bps,
            diag_frames: 0,
            last_quality_report: Instant::now(),
            community_id: params.community_id,
            channel_id: params.channel_id,
            our_pseudonym: params.our_pseudonym,
            report_rx: params.report_rx,
            peer_links: HashMap::new(),
            media_liveness: params.media_liveness,
            media: None,
        })
    }

    async fn run_loop(mut self) {
        tracing::info!("voice send loop started");
        let mut allocation = {
            let transport = self.transport.lock().await;
            self.media = Some(transport.media());
            transport.allocator().subscribe()
        };
        loop {
            tokio::select! {
                biased;
                () = self.stop.cancelled() => {
                    tracing::info!("voice send loop: shutdown signal received");
                    break;
                }
                maybe = self.capture_rx.recv() => {
                    let Some(samples) = maybe else {
                        tracing::info!("voice send loop: capture channel closed");
                        break;
                    };
                    self.process_samples(samples).await;
                }
                Some(report) = self.report_rx.recv() => {
                    self.note_receiver_report(&report);
                }
                Ok(()) = allocation.changed() => {
                    let audio_bps = allocation.borrow_and_update().audio_bps;
                    self.apply_audio_bitrate(audio_bps);
                }
            }
        }
        self.cleanup();
    }

    /// Architecture §10.7: outside a stage channel, every member may
    /// transmit. Inside a stage channel, only the listed speakers may.
    fn is_allowed_to_transmit(&self) -> bool {
        let Some(ref community_id) = self.community_id else {
            return true;
        };
        if !self.deps.channel_is_stage(community_id, &self.channel_id) {
            return true;
        }
        let Some(ref my_pseudonym) = self.our_pseudonym else {
            return false;
        };
        self.deps
            .we_are_stage_speaker(community_id, &self.channel_id, my_pseudonym)
    }

    async fn process_samples(&mut self, samples: Vec<f32>) {
        self.pcm_buffer.extend_from_slice(&samples);
        while self.pcm_buffer.len() >= self.frame_size {
            let frame: Vec<f32> = self.pcm_buffer.drain(..self.frame_size).collect();
            self.process_frame(frame).await;
        }
    }

    async fn process_frame(&mut self, frame_samples: Vec<f32>) {
        // Stage gate: audience members drain capture but never encode.
        if !self.is_allowed_to_transmit() {
            self.flip_speaking_off_if_needed();
            return;
        }

        // Skip processing when muted — still drain capture to avoid backpressure.
        if self.muted_flag.load(Ordering::Relaxed) {
            self.flip_speaking_off_if_needed();
            return;
        }

        // Drain speaker reference frames for AEC.
        let mut latest_speaker_ref: Option<Vec<f32>> = None;
        while let Ok(ref_frame) = self.speaker_ref_rx.try_recv() {
            self.processor.feed_speaker_reference(&ref_frame);
            latest_speaker_ref = Some(ref_frame);
        }

        // Run audio processor (AEC + denoise + VAD).
        let processed = self
            .processor
            .process_capture(&frame_samples, latest_speaker_ref.as_deref());

        // Diagnostic telemetry (~every 2s). Distinguishes the three ways
        // outbound voice silently dies: a silent/wrong capture device
        // (raw_peak ~0), AEC/denoise over-suppression (raw_peak healthy but
        // proc_peak ~0), and VAD gating (both peaks healthy but is_speech
        // false → nothing clears the gate below). `packets_sent` shows whether
        // anything is actually leaving the node.
        self.diag_frames = self.diag_frames.wrapping_add(1);
        if self.diag_frames.is_multiple_of(100) {
            let raw_peak = frame_samples.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
            let proc_peak = processed
                .samples
                .iter()
                .fold(0.0f32, |m, &s| m.max(s.abs()));
            tracing::debug!(
                raw_peak,
                proc_peak,
                is_speech = processed.is_speech,
                had_speaker_ref = latest_speaker_ref.is_some(),
                packets_sent = self.packets_sent,
                "voice capture diagnostic"
            );
        }

        // Emit speaking state change to frontend.
        if processed.is_speech != self.was_speaking {
            self.was_speaking = processed.is_speech;
            self.deps.emit_voice_event(VoiceSessionEvent::UserSpeaking {
                peer_pubkey: self.public_key.clone(),
                speaking: processed.is_speech,
            });
        }

        // Only encode and send if speaking (VAD gate).
        if !processed.is_speech {
            return;
        }

        let mut encoded = match self.codec.encode(&processed.samples) {
            Ok(frame) => frame,
            Err(e) => {
                tracing::warn!(error = %e, "voice send loop: Opus encode failed");
                return;
            }
        };

        encoded.sequence = self.sequence;
        encoded.timestamp = rekindle_utils::timestamp_ms();
        self.sequence = self.sequence.wrapping_add(1);

        {
            let transport = self.transport.lock().await;
            if transport.is_connected() {
                // No key for the scope yet → DROP: voice never leaves this
                // node in the clear.
                let Some(sframe) = self.sealer.seal(
                    transport.sender_key(),
                    encoded.sequence,
                    encoded.timestamp,
                    &encoded.data,
                ) else {
                    tracing::debug!(
                        channel = %self.channel_id,
                        "no media key yet — voice frame dropped (never sent plaintext)"
                    );
                    return;
                };
                let frame = OutboundFrame {
                    sequence: encoded.sequence,
                    timestamp: encoded.timestamp,
                    sframe,
                    media_bytes: encoded.data.len(),
                };
                // Queued on each route's pacer; the routes' egress drivers
                // deliver and log failures (plan E4.3.3). There is no
                // sender-side route repair: a call's media route travels
                // only in voice signaling, and its owner re-announces it
                // when Veilid reports it dead (plans C7.15, C7.23).
                let queued = match transport.mode() {
                    VoiceMode::Mesh => transport.broadcast(&frame),
                    VoiceMode::Mcu { ref host_pseudonym } if *host_pseudonym == self.public_key => {
                        // We are the MCU host — MCU loop handles mixing + distribution.
                        Ok(())
                    }
                    // Non-host: send only to the MCU host.
                    VoiceMode::Mcu { ref host_pseudonym } => {
                        transport.send_to_peer(host_pseudonym, &frame)
                    }
                };
                drop(transport);
                match queued {
                    Ok(()) => self.packets_sent += 1,
                    Err(e) => tracing::warn!(error = %e, "voice send loop: frame not queued"),
                }
            }
        }

        self.report_quality_if_due();
    }

    fn flip_speaking_off_if_needed(&mut self) {
        if self.was_speaking {
            self.was_speaking = false;
            self.deps.emit_voice_event(VoiceSessionEvent::UserSpeaking {
                peer_pubkey: self.public_key.clone(),
                speaking: false,
            });
        }
    }

    fn cleanup(self) {
        // Transport disconnect is handled by shutdown_voice — we don't
        // clear the shared transport here since other loops or handlers
        // may still use it.
        self.log_route_summaries();
        if self.was_speaking {
            self.deps.emit_voice_event(VoiceSessionEvent::UserSpeaking {
                peer_pubkey: self.public_key,
                speaking: false,
            });
        }
        tracing::info!("voice send loop exited");
    }
}

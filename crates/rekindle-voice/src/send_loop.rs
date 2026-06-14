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

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use tokio::sync::{broadcast, mpsc};

use crate::audio_processing::AudioProcessor;
use crate::codec::OpusCodec;
use crate::session_deps::{VoiceSessionDeps, VoiceSessionEvent};
use crate::transport::VoiceTransport;
use crate::VoiceMode;

/// Consecutive per-peer send failures before the route-heal hook fires
/// (and the warn re-logs). 50 ≈ one second of speech at 20 ms frames.
const ROUTE_HEAL_THRESHOLD: u64 = 50;

pub struct VoiceSendParams {
    pub capture_rx: Option<mpsc::Receiver<Vec<f32>>>,
    pub transport: Arc<tokio::sync::Mutex<VoiceTransport>>,
    pub shutdown_rx: mpsc::Receiver<()>,
    pub deps: Arc<dyn VoiceSessionDeps>,
    pub public_key: String,
    pub noise_suppression: bool,
    pub echo_cancellation: bool,
    pub muted_flag: Arc<AtomicBool>,
    pub speaker_ref_rx: broadcast::Receiver<Vec<f32>>,
    /// Community ID for MEK encryption. `None` for 1:1 calls.
    pub community_id: Option<String>,
    /// Voice channel ID we're transmitting in. Used with the stage
    /// gate (§10.7) to drop frames from non-speakers.
    pub channel_id: String,
    /// Our pseudonym in this community (for stage-speaker check).
    /// `None` for 1:1 calls (no stage gate applies).
    pub our_pseudonym: Option<String>,
}

struct VoiceSendLoop {
    capture_rx: mpsc::Receiver<Vec<f32>>,
    transport: Arc<tokio::sync::Mutex<VoiceTransport>>,
    shutdown_rx: mpsc::Receiver<()>,
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
    send_failures: u64,
    /// Diagnostic: frames seen by `process_frame`, for periodic capture
    /// level + VAD logging (the only outbound-voice observability we have).
    diag_frames: u64,
    /// Consecutive send failures per peer — drives the route-heal
    /// escalation (a dead remote route fails every frame; without
    /// healing it is hammered 50×/s forever).
    peer_send_failures: HashMap<String, u64>,
    last_quality_report: Instant,
    community_id: Option<String>,
    channel_id: String,
    our_pseudonym: Option<String>,
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

        let sample_rate: u32 = 48000;
        let channels: u16 = 1;
        let frame_size: usize = 960; // 20ms at 48kHz

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

        Some(Self {
            capture_rx,
            transport: params.transport,
            shutdown_rx: params.shutdown_rx,
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
            send_failures: 0,
            diag_frames: 0,
            peer_send_failures: HashMap::new(),
            last_quality_report: Instant::now(),
            community_id: params.community_id,
            channel_id: params.channel_id,
            our_pseudonym: params.our_pseudonym,
        })
    }

    async fn run_loop(mut self) {
        tracing::info!("voice send loop started");
        loop {
            tokio::select! {
                biased;
                _ = self.shutdown_rx.recv() => {
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

        // Encrypt with the channel-media MEK (§10.5 hierarchy: channel
        // MEK when the join/leave rotation distributed one, community
        // MEK otherwise). The generation rides the wire so receivers
        // detect rotation races instead of decrypt-failing blind. NO
        // key → DROP the frame: community voice must never leave this
        // node in plaintext (the old `if let Some` silently skipped
        // encryption when the cache was empty).
        if let Some(ref cid) = self.community_id {
            let Some((mek_bytes, generation)) = self.deps.channel_media_mek(cid, &self.channel_id)
            else {
                tracing::warn!(
                    community = %cid,
                    channel = %self.channel_id,
                    "no channel-media MEK — voice frame dropped (never sent plaintext)"
                );
                return;
            };
            let mek = MediaEncryptionKey::from_bytes(mek_bytes, generation);
            match mek.encrypt(&encoded.data) {
                Ok(ciphertext) => {
                    encoded.data = ciphertext;
                    encoded.mek_generation = generation;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "voice MEK encrypt failed");
                    return;
                }
            }
        }

        {
            let transport = self.transport.lock().await;
            if transport.is_connected() {
                let (roster, errors) = match transport.mode() {
                    VoiceMode::Mesh => {
                        let roster = transport.peer_keys();
                        let errors = transport.broadcast(&encoded).await;
                        (roster, errors)
                    }
                    VoiceMode::Mcu { ref host_pseudonym } if *host_pseudonym == self.public_key => {
                        // We are the MCU host — MCU loop handles mixing + distribution.
                        (Vec::new(), Vec::new())
                    }
                    VoiceMode::Mcu { ref host_pseudonym } => {
                        // Non-host: send only to the MCU host.
                        let roster = vec![host_pseudonym.clone()];
                        let errors = match transport.send_to_peer(host_pseudonym, &encoded).await {
                            Ok(()) => Vec::new(),
                            Err(e) => vec![(host_pseudonym.clone(), e)],
                        };
                        (roster, errors)
                    }
                };
                drop(transport);
                self.note_send_results(&roster, &errors);
                self.packets_sent += 1;
            }
        }

        self.report_quality_if_due();
    }

    /// Per-peer send accounting: success resets the failure streak; a
    /// streak hitting multiples of [`ROUTE_HEAL_THRESHOLD`] fires the
    /// deps route-heal hook (presence re-resolve → roster refresh) and
    /// the rate-limited warn — NOT one log line per frame per peer.
    /// The whole-frame `send_failures` counter (quality classification)
    /// counts a frame failed only when EVERY peer failed, preserving
    /// the original loss semantics.
    fn note_send_results(
        &mut self,
        roster: &[String],
        errors: &[(String, crate::error::VoiceError)],
    ) {
        if !roster.is_empty() && !errors.is_empty() && errors.len() >= roster.len() {
            self.send_failures += 1;
        }
        let failed: std::collections::HashSet<&str> =
            errors.iter().map(|(k, _)| k.as_str()).collect();
        for key in roster {
            if !failed.contains(key.as_str()) {
                self.peer_send_failures.remove(key);
            }
        }
        for (key, error) in errors {
            // "*" is the transport's not-initialized sentinel, not a peer.
            if key == "*" {
                tracing::warn!(error = %error, "voice send loop: transport send failed");
                continue;
            }
            let n = self.peer_send_failures.entry(key.clone()).or_insert(0);
            *n += 1;
            if *n == 1 || n.is_multiple_of(ROUTE_HEAL_THRESHOLD) {
                tracing::warn!(
                    peer = %key,
                    consecutive_failures = *n,
                    error = %error,
                    "voice send failing for peer"
                );
            }
            if n.is_multiple_of(ROUTE_HEAL_THRESHOLD) {
                self.spawn_route_heal(key.clone());
            }
        }
    }

    /// Crate-side route heal — the voice mirror of gossip's
    /// `send_to_one_peer` re-resolve. Spawned off the 20 ms hot path:
    /// resolve the peer's CURRENT route through the deps port, then
    /// refresh the transport roster entry (refresh-only: a peer who
    /// left must not be re-added). Community sessions only — DM 1:1
    /// call routes heal via call signaling.
    fn spawn_route_heal(&self, peer: String) {
        let Some(community_id) = self.community_id.clone() else {
            return;
        };
        let deps = Arc::clone(&self.deps);
        let transport = Arc::clone(&self.transport);
        let handle = tokio::spawn(async move {
            let Some(fresh) = deps.resolve_peer_route_from_dht(&community_id, &peer).await else {
                tracing::warn!(
                    community = %community_id,
                    peer = %peer,
                    "voice route heal: no fresh route in presence registry"
                );
                return;
            };
            if fresh.is_empty() {
                return;
            }
            let refreshed = transport.lock().await.refresh_peer_route(&peer, &fresh);
            tracing::info!(
                community = %community_id,
                peer = %peer,
                refreshed,
                "voice route heal: presence re-resolve applied"
            );
        });
        self.deps.register_background_handle(handle);
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

    fn report_quality_if_due(&mut self) {
        if self.last_quality_report.elapsed() < Duration::from_secs(5) {
            return;
        }

        let loss_pct_u32 = self
            .send_failures
            .saturating_mul(100)
            .checked_div(self.packets_sent)
            .and_then(|loss| u32::try_from(loss).ok())
            .unwrap_or(0);
        let quality = match loss_pct_u32 {
            0..5 => "good",
            5..15 => "fair",
            _ => "poor",
        };
        self.deps
            .emit_voice_event(VoiceSessionEvent::ConnectionQuality {
                quality: quality.to_string(),
            });

        // Update Opus FEC based on measured loss.
        let loss_i32 = i32::try_from(loss_pct_u32.min(100)).unwrap_or(100);
        let _ = self.codec.set_packet_loss_perc(loss_i32);

        // Adaptive bitrate based on group size. Cannot hold the tokio
        // Mutex synchronously, so use try_lock.
        if let Ok(transport) = self.transport.try_lock() {
            let peer_count = transport.peer_count();
            let target_bps = match peer_count {
                0..=2 => 32000,
                3..=7 => 24000,
                _ => 16000,
            };
            let _ = self.codec.set_bitrate(target_bps);
        }

        self.packets_sent = 0;
        self.send_failures = 0;
        self.last_quality_report = Instant::now();
    }

    fn cleanup(self) {
        // Transport disconnect is handled by shutdown_voice — we don't
        // clear the shared transport here since other loops or handlers
        // may still use it.
        if self.was_speaking {
            self.deps.emit_voice_event(VoiceSessionEvent::UserSpeaking {
                peer_pubkey: self.public_key,
                speaking: false,
            });
        }
        tracing::info!("voice send loop exited");
    }
}

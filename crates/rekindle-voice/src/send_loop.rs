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

        // Encrypt voice frame with community MEK (if in a community channel).
        if let Some(ref cid) = self.community_id {
            if let Some(mek_bytes) = self.deps.community_voice_mek(cid) {
                // MEK generation 0 here — the cache already returns the
                // current generation's MEK; the receiver's MEK chain
                // tracks generation independently.
                let mek = MediaEncryptionKey::from_bytes(mek_bytes, 0);
                match mek.encrypt(&encoded.data) {
                    Ok(ciphertext) => encoded.data = ciphertext,
                    Err(e) => {
                        tracing::warn!(error = %e, "voice MEK encrypt failed");
                        return;
                    }
                }
            }
        }

        {
            let transport = self.transport.lock().await;
            if transport.is_connected() {
                let send_result = match transport.mode() {
                    VoiceMode::Mesh => transport.send(&encoded).await,
                    VoiceMode::Mcu { ref host_pseudonym }
                        if *host_pseudonym == self.public_key =>
                    {
                        // We are the MCU host — MCU loop handles mixing + distribution.
                        Ok(())
                    }
                    VoiceMode::Mcu { ref host_pseudonym } => {
                        // Non-host: send only to the MCU host.
                        transport.send_to_peer(host_pseudonym, &encoded).await
                    }
                };
                if let Err(e) = send_result {
                    tracing::warn!(error = %e, "voice send loop: transport send failed");
                    self.send_failures += 1;
                }
                self.packets_sent += 1;
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
        self.deps.emit_voice_event(VoiceSessionEvent::ConnectionQuality {
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

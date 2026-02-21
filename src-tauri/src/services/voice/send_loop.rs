//! Voice capture → process → encode → transport send pipeline.
//!
//! The send loop drains `capture_rx`, runs `AudioProcessor` (AEC + denoise + VAD),
//! encodes with Opus, and sends via the `VoiceTransport`. It owns the transport and
//! runs until a shutdown signal or the capture channel closes.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tauri::Emitter;
use tokio::sync::{broadcast, mpsc};

use crate::channels::VoiceEvent;

pub(crate) struct VoiceSendParams {
    pub capture_rx: Option<mpsc::Receiver<Vec<f32>>>,
    pub transport: rekindle_voice::transport::VoiceTransport,
    pub shutdown_rx: mpsc::Receiver<()>,
    pub app: tauri::AppHandle,
    pub public_key: String,
    pub noise_suppression: bool,
    pub echo_cancellation: bool,
    pub muted_flag: Arc<AtomicBool>,
    pub speaker_ref_rx: broadcast::Receiver<Vec<f32>>,
}

struct VoiceSendLoop {
    capture_rx: mpsc::Receiver<Vec<f32>>,
    transport: rekindle_voice::transport::VoiceTransport,
    shutdown_rx: mpsc::Receiver<()>,
    app: tauri::AppHandle,
    public_key: String,
    codec: rekindle_voice::codec::OpusCodec,
    processor: rekindle_voice::audio_processing::AudioProcessor,
    muted_flag: Arc<AtomicBool>,
    speaker_ref_rx: broadcast::Receiver<Vec<f32>>,
    pcm_buffer: Vec<f32>,
    frame_size: usize,
    sequence: u32,
    was_speaking: bool,
    packets_sent: u64,
    send_failures: u64,
    last_quality_report: Instant,
}

/// Entry point: validate params, build loop state, run until shutdown.
pub(crate) async fn run(params: VoiceSendParams) {
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

        let codec = match rekindle_voice::codec::OpusCodec::new(sample_rate, channels, frame_size) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(error = %e, "voice send loop: failed to create Opus codec");
                return None;
            }
        };

        let frame_duration_ms = u32::try_from(frame_size).unwrap_or(960) * 1000 / sample_rate;
        let processor = rekindle_voice::audio_processing::AudioProcessor::new(
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
            app: params.app,
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

    async fn process_samples(&mut self, samples: Vec<f32>) {
        self.pcm_buffer.extend_from_slice(&samples);
        while self.pcm_buffer.len() >= self.frame_size {
            let frame: Vec<f32> = self.pcm_buffer.drain(..self.frame_size).collect();
            self.process_frame(frame).await;
        }
    }

    async fn process_frame(&mut self, frame_samples: Vec<f32>) {
        // Skip processing when muted — still drain capture to avoid backpressure
        if self.muted_flag.load(Ordering::Relaxed) {
            if self.was_speaking {
                self.was_speaking = false;
                let event = VoiceEvent::UserSpeaking {
                    public_key: self.public_key.clone(),
                    speaking: false,
                };
                let _ = self.app.emit("voice-event", &event);
            }
            return;
        }

        // Drain speaker reference frames for AEC
        let mut latest_speaker_ref: Option<Vec<f32>> = None;
        while let Ok(ref_frame) = self.speaker_ref_rx.try_recv() {
            self.processor.feed_speaker_reference(&ref_frame);
            latest_speaker_ref = Some(ref_frame);
        }

        // Run audio processor (AEC + denoise + VAD)
        let processed = self
            .processor
            .process_capture(&frame_samples, latest_speaker_ref.as_deref());

        // Emit speaking state change to frontend
        if processed.is_speech != self.was_speaking {
            self.was_speaking = processed.is_speech;
            let event = VoiceEvent::UserSpeaking {
                public_key: self.public_key.clone(),
                speaking: processed.is_speech,
            };
            let _ = self.app.emit("voice-event", &event);
        }

        // Only encode and send if speaking (VAD gate)
        if !processed.is_speech {
            return;
        }

        // Encode the processed PCM frame with Opus
        let mut encoded = match self.codec.encode(&processed.samples) {
            Ok(frame) => frame,
            Err(e) => {
                tracing::warn!(error = %e, "voice send loop: Opus encode failed");
                return;
            }
        };

        encoded.sequence = self.sequence;
        encoded.timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0);
        self.sequence = self.sequence.wrapping_add(1);

        if self.transport.is_connected() {
            if let Err(e) = self.transport.send(&encoded).await {
                tracing::debug!(error = %e, "voice send loop: transport send failed");
                self.send_failures += 1;
            }
            self.packets_sent += 1;
        }

        self.report_quality_if_due();
    }

    fn report_quality_if_due(&mut self) {
        if self.last_quality_report.elapsed() < Duration::from_secs(5) {
            return;
        }

        let loss_pct = if self.packets_sent > 0 {
            #[allow(clippy::cast_precision_loss)]
            let pct = (self.send_failures as f64 / self.packets_sent as f64) * 100.0;
            pct
        } else {
            0.0
        };
        let quality = if loss_pct < 5.0 {
            "good"
        } else if loss_pct < 15.0 {
            "fair"
        } else {
            "poor"
        };
        let event = VoiceEvent::ConnectionQuality {
            quality: quality.to_string(),
        };
        let _ = self.app.emit("voice-event", &event);

        // Update Opus FEC based on measured loss
        #[allow(clippy::cast_possible_truncation)]
        let loss_i32 = (loss_pct as i32).clamp(0, 100);
        let _ = self.codec.set_packet_loss_perc(loss_i32);

        self.packets_sent = 0;
        self.send_failures = 0;
        self.last_quality_report = Instant::now();
    }

    fn cleanup(mut self) {
        if let Err(e) = self.transport.disconnect() {
            tracing::warn!(error = %e, "voice send loop: transport disconnect failed");
        }

        if self.was_speaking {
            let event = VoiceEvent::UserSpeaking {
                public_key: self.public_key,
                speaking: false,
            };
            let _ = self.app.emit("voice-event", &event);
        }

        tracing::info!("voice send loop exited");
    }
}

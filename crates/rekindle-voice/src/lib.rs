// Veilid + tokio + tracing nested-future Send bound evaluation can
// overflow the default recursion limit (256 is the rustc default; the
// MCU loop's tokio::spawn inside a transport.lock().await triggers
// the chain). Bump high enough to clear the Send chain for the deepest
// async future.
#![recursion_limit = "512"]

pub mod audio_processing;
pub(crate) mod audio_thread;
pub mod capture;
pub mod codec;
pub mod device;
pub mod election; // Phase 14 — deterministic MCU host election (pure logic).
pub mod error;
pub mod jitter;
pub mod mcu_loop; // Phase 14 — MCU mixing for groups (>4 participants or stage channels).
pub mod mixer;
pub mod playback;
pub mod receive_loop; // Phase 14 — packet receive → decode → mix → playback pipeline.
pub mod replay_window;
pub mod send_loop; // Phase 14 — capture → process → encode → transport send pipeline.
pub mod session; // Phase 14.l — voice session orchestrator (start_session port).
pub mod session_deps; // Phase 14 — VoiceSessionDeps trait + VoiceSessionEvent.
pub mod signaling; // Phase 14.k — community voice signaling handlers (VoiceSignalingDeps).
pub mod topology; // Phase 14 — pure mode-decision + stage-host election math.
pub mod transport;

pub use election::{channel_target, elect_relay_host};
pub use error::VoiceError;
pub use session_deps::{
    AudioPrefs, CallKeyInfo, VoiceIdentity, VoicePeer, VoiceSessionDeps, VoiceSessionEvent,
    VoiceSessionStartup, VoiceShutdownHandles, VoiceShutdownOpts,
};
pub use transport::VoiceMode;

use tokio::sync::mpsc;

use crate::capture::AudioCapture;
use crate::codec::OpusCodec;
use crate::jitter::JitterBuffer;
use crate::mixer::AudioMixer;
use crate::playback::AudioPlayback;

/// Configuration for the voice engine.
pub struct VoiceConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub frame_size: usize,
    pub jitter_buffer_ms: u32,
    pub vad_threshold: f32,
    pub vad_hold_ms: u32,
    /// Whether noise suppression (nnnoiseless/RNNoise) is enabled.
    pub noise_suppression: bool,
    /// Whether echo cancellation (AEC3) is enabled.
    pub echo_cancellation: bool,
    /// Selected input device name (None = system default).
    pub input_device: Option<String>,
    /// Selected output device name (None = system default).
    pub output_device: Option<String>,
    /// Input volume multiplier (0.0–1.0).
    pub input_volume: f32,
    /// Output volume multiplier (0.0–1.0).
    pub output_volume: f32,
}

/// Returns recommended voice settings based on group size (9E: Adaptive Codec).
///
/// Smaller groups get higher bitrate and lower jitter for clarity.
/// Larger groups grow the jitter target so MCU mixing can absorb
/// per-source desynchronisation; the latency cost is acceptable
/// because large-group calls are typically meeting-style rather than
/// duplex conversation.
pub fn voice_config_for_group_size(n: usize) -> VoiceConfig {
    match n {
        0..=3 => VoiceConfig {
            // Small groups (1:1 calls and 2-3 person huddles): keep
            // the default low-latency jitter target so mouth-to-ear
            // stays under the architecture §32 line 4147 budget.
            ..VoiceConfig::default()
        },
        4..=8 => VoiceConfig {
            // 80ms jitter — accepts ~40ms more latency to absorb
            // multi-source MCU desynchronisation.
            jitter_buffer_ms: 80,
            ..VoiceConfig::default()
        },
        _ => VoiceConfig {
            // 9+ participants: 120ms target trades latency for
            // glitch-free mixing at scale. Spec target may not be
            // met for these sessions; group-call UX is meeting-style
            // rather than realtime duplex, so the trade is acceptable.
            jitter_buffer_ms: 120,
            ..VoiceConfig::default()
        },
    }
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            channels: 1,
            frame_size: 960, // 20ms at 48kHz
            // Architecture §32 Phase 7 W26 line 4147 ("<100ms
            // mouth-to-ear") leaves room for ~40ms of jitter buffer
            // after capture + Opus algorithmic + decode + playback
            // overhead. 40ms matches the industry VoIP defaults
            // (Mumble 20–50ms, Discord ~40ms, WebRTC ~50ms) and is
            // achievable over Veilid's `SafetySelection::Unsafe`
            // per-packet routes used by voice. Adaptive jitter
            // (start small, grow on observed loss) is the proper
            // long-term solution; until then 40ms is the
            // spec-compliant default.
            jitter_buffer_ms: 40,
            vad_threshold: 0.02,
            vad_hold_ms: 300,
            noise_suppression: true,
            echo_cancellation: true,
            input_device: None,
            output_device: None,
            input_volume: 1.0,
            output_volume: 1.0,
        }
    }
}

/// Main voice engine coordinating capture, encoding, transport, and playback.
///
/// Audio pipeline:
///
/// ```text
/// Microphone ──cpal──▶ capture_rx ──▶ AudioProcessor + Opus encode ──▶ Transport (Veilid)
///                                                                          │
///                                                                          ▼
/// Speaker ◀──cpal── playback_tx ◀── Opus decode + mix ◀── Jitter buffer ◀─┘
/// ```
///
/// `start_capture` and `start_playback` open real audio devices via cpal and
/// create the mpsc channels that connect them to the encode/decode pipeline.
/// The full processing loop (chunking into Opus frames, encoding, transport,
/// jitter buffering, decoding, mixing) is driven externally once transport
/// is available.
pub struct VoiceEngine {
    pub codec: OpusCodec,
    pub capture: Option<AudioCapture>,
    pub playback: Option<AudioPlayback>,
    pub jitter_buffer: JitterBuffer,
    pub mixer: AudioMixer,
    pub is_muted: bool,
    pub is_deafened: bool,

    /// Receiver end of the capture channel — raw PCM chunks from the mic.
    /// A processing loop should drain this, run `AudioProcessor`, chunk into
    /// `frame_size` blocks, and encode with `codec.encode()`.
    capture_rx: Option<mpsc::Receiver<Vec<f32>>>,

    /// Sender end of the playback channel — decoded PCM chunks to the speaker.
    /// A processing loop should decode incoming packets and send mixed audio here.
    playback_tx: Option<mpsc::Sender<Vec<f32>>>,

    /// Merged device error receiver — capture and playback errors both funnel here.
    /// Taken by the device monitor loop via `take_device_error_rx()`.
    device_error_rx: Option<mpsc::Receiver<String>>,

    /// Shared sender for device errors — cloned for capture and playback error callbacks.
    device_error_tx: Option<mpsc::Sender<String>>,

    /// Saved config for creating capture/playback instances.
    config: VoiceConfig,
}

impl VoiceEngine {
    /// Create a new voice engine with the given configuration.
    pub fn new(config: VoiceConfig) -> Result<Self, VoiceError> {
        let codec = OpusCodec::new(config.sample_rate, config.channels, config.frame_size)?;
        let (device_error_tx, device_error_rx) = mpsc::channel::<String>(16);

        Ok(Self {
            codec,
            capture: None,
            playback: None,
            jitter_buffer: JitterBuffer::new(config.jitter_buffer_ms),
            mixer: AudioMixer::new(config.channels),
            is_muted: false,
            is_deafened: false,
            capture_rx: None,
            playback_tx: None,
            device_error_rx: Some(device_error_rx),
            device_error_tx: Some(device_error_tx),
            config,
        })
    }

    /// Start audio capture from the microphone.
    ///
    /// Opens the selected (or default) input device via cpal and begins streaming
    /// PCM chunks into an internal mpsc channel. Use `take_capture_rx()` to get
    /// the receiver for a processing task.
    pub fn start_capture(&mut self) -> Result<(), VoiceError> {
        // Channel capacity: ~100 frames ≈ 2 seconds of audio at 20ms per frame.
        let (tx, rx) = mpsc::channel::<Vec<f32>>(100);

        let mut capture = AudioCapture::new(self.config.sample_rate, self.config.channels);
        capture.start(
            tx,
            self.config.input_device.as_deref(),
            self.device_error_tx.clone(),
        )?;

        self.capture = Some(capture);
        self.capture_rx = Some(rx);
        tracing::info!("voice capture pipeline started");
        Ok(())
    }

    /// Stop audio capture.
    pub fn stop_capture(&mut self) {
        if let Some(ref mut capture) = self.capture {
            capture.stop();
        }
        self.capture = None;
        self.capture_rx = None;
    }

    /// Start audio playback to the speaker.
    ///
    /// Opens the selected (or default) output device via cpal. Decoded/mixed PCM
    /// chunks should be sent to the sender returned by `take_playback_tx()`.
    pub fn start_playback(&mut self) -> Result<(), VoiceError> {
        let (tx, rx) = mpsc::channel::<Vec<f32>>(100);

        let mut pb = AudioPlayback::new(self.config.sample_rate, self.config.channels);
        pb.start(
            rx,
            self.config.output_device.as_deref(),
            self.device_error_tx.clone(),
        )?;

        self.playback = Some(pb);
        self.playback_tx = Some(tx);
        tracing::info!("voice playback pipeline started");
        Ok(())
    }

    /// Stop audio playback.
    pub fn stop_playback(&mut self) {
        if let Some(ref mut pb) = self.playback {
            pb.stop();
        }
        self.playback = None;
        self.playback_tx = None;
    }

    /// Take the capture receiver to use in a processing task.
    pub fn take_capture_rx(&mut self) -> Option<mpsc::Receiver<Vec<f32>>> {
        self.capture_rx.take()
    }

    /// Take the playback sender to use in a processing task.
    pub fn take_playback_tx(&mut self) -> Option<mpsc::Sender<Vec<f32>>> {
        self.playback_tx.take()
    }

    /// Process an incoming voice packet from the network.
    pub fn process_incoming(&mut self, packet: transport::VoicePacket) {
        self.jitter_buffer.push(packet);
    }

    /// Set mute state (flag only — does NOT stop capture device).
    pub fn set_muted(&mut self, muted: bool) {
        self.is_muted = muted;
    }

    /// Set deafen state (flag only — does NOT stop playback device).
    pub fn set_deafened(&mut self, deafened: bool) {
        self.is_deafened = deafened;
    }

    /// Get the current `VoiceConfig`.
    pub fn config(&self) -> &VoiceConfig {
        &self.config
    }

    /// Update the audio device names in the config.
    ///
    /// Takes effect on the next `start_capture`/`start_playback` call.
    pub fn set_devices(&mut self, input_device: Option<String>, output_device: Option<String>) {
        self.config.input_device = input_device;
        self.config.output_device = output_device;
    }

    /// Take the device error receiver for use in a device monitor loop.
    ///
    /// Returns `None` if already taken or never created.
    pub fn take_device_error_rx(&mut self) -> Option<mpsc::Receiver<String>> {
        self.device_error_rx.take()
    }

    /// Create fresh device error channels (used after restart when the old
    /// receiver was consumed by a previous monitor loop).
    pub fn refresh_device_error_channels(&mut self) -> mpsc::Receiver<String> {
        let (tx, rx) = mpsc::channel::<String>(16);
        self.device_error_tx = Some(tx);
        rx
    }
}

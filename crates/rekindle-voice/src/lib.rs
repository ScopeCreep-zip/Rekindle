pub mod capture;
pub mod codec;
pub mod error;
pub mod jitter;
pub mod mixer;
pub mod playback;
pub mod transport;
pub mod vad;

pub use error::VoiceError;

use tokio::sync::mpsc;

use crate::capture::AudioCapture;
use crate::codec::OpusCodec;
use crate::jitter::JitterBuffer;
use crate::mixer::AudioMixer;
use crate::playback::AudioPlayback;
use crate::transport::VoiceTransport;
use crate::vad::VoiceActivityDetector;

/// Configuration for the voice engine.
pub struct VoiceConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub frame_size: usize,
    pub jitter_buffer_ms: u32,
    pub vad_threshold: f32,
    pub vad_hold_ms: u32,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            channels: 1,
            frame_size: 960, // 20ms at 48kHz
            jitter_buffer_ms: 60,
            vad_threshold: 0.02,
            vad_hold_ms: 300,
        }
    }
}

/// Main voice engine coordinating capture, encoding, transport, and playback.
///
/// Audio pipeline:
///
/// ```text
/// Microphone ──cpal──▶ capture_rx ──▶ VAD + Opus encode ──▶ Transport (Veilid)
///                                                               │
///                                                               ▼
/// Speaker ◀──cpal── playback_tx ◀── Opus decode + mix ◀── Jitter buffer
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
    pub transport: Option<VoiceTransport>,
    pub jitter_buffer: JitterBuffer,
    pub mixer: AudioMixer,
    pub vad: VoiceActivityDetector,
    pub is_muted: bool,
    pub is_deafened: bool,

    /// Receiver end of the capture channel — raw PCM chunks from the mic.
    /// A processing loop should drain this, run VAD, chunk into `frame_size`
    /// blocks, and encode with `codec.encode()`.
    capture_rx: Option<mpsc::Receiver<Vec<f32>>>,

    /// Sender end of the playback channel — decoded PCM chunks to the speaker.
    /// A processing loop should decode incoming packets and send mixed audio here.
    playback_tx: Option<mpsc::Sender<Vec<f32>>>,

    /// Saved config for creating capture/playback instances.
    config: VoiceConfig,
}

impl VoiceEngine {
    /// Create a new voice engine with the given configuration.
    pub fn new(config: VoiceConfig) -> Result<Self, VoiceError> {
        let frame_size_u32 = u32::try_from(config.frame_size)
            .expect("frame_size must fit in u32");
        let frame_duration_ms = (frame_size_u32 * 1000) / config.sample_rate;
        let codec = OpusCodec::new(config.sample_rate, config.channels, config.frame_size)?;

        Ok(Self {
            codec,
            capture: None,
            playback: None,
            transport: None,
            jitter_buffer: JitterBuffer::new(config.jitter_buffer_ms),
            mixer: AudioMixer::new(config.channels),
            vad: VoiceActivityDetector::new(
                config.vad_threshold,
                config.vad_hold_ms,
                frame_duration_ms,
            ),
            is_muted: false,
            is_deafened: false,
            capture_rx: None,
            playback_tx: None,
            config,
        })
    }

    /// Start audio capture from the microphone.
    ///
    /// Opens the default input device via cpal and begins streaming PCM chunks
    /// into an internal mpsc channel. Use `take_capture_rx()` to get the
    /// receiver for a processing task.
    pub fn start_capture(&mut self) -> Result<(), VoiceError> {
        // Channel capacity: ~100 frames ≈ 2 seconds of audio at 20ms per frame.
        // try_send in the audio callback will drop frames if the consumer falls
        // behind, which is acceptable for real-time voice.
        let (tx, rx) = mpsc::channel::<Vec<f32>>(100);

        let mut capture =
            AudioCapture::new(self.config.sample_rate, self.config.channels)?;
        capture.start(tx)?;

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
    /// Opens the default output device via cpal. Decoded/mixed PCM chunks
    /// should be sent to the sender returned by `take_playback_tx()`.
    pub fn start_playback(&mut self) -> Result<(), VoiceError> {
        let (tx, rx) = mpsc::channel::<Vec<f32>>(100);

        let mut pb = AudioPlayback::new(self.config.sample_rate, self.config.channels)?;
        pb.start(rx)?;

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
    ///
    /// Returns `None` if capture hasn't been started or the receiver was
    /// already taken.
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

    /// Set mute state.
    pub fn set_muted(&mut self, muted: bool) {
        self.is_muted = muted;
        if muted {
            self.stop_capture();
        }
    }

    /// Set deafen state.
    pub fn set_deafened(&mut self, deafened: bool) {
        self.is_deafened = deafened;
        if deafened {
            self.stop_playback();
        }
    }
}

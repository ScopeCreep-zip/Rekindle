use std::collections::VecDeque;
use std::sync::mpsc as std_mpsc;

use cpal::traits::DeviceTrait;
use tokio::sync::mpsc;

use crate::audio_thread::{AudioThread, AudioThreadLabels};
use crate::device::{resolve_device, DeviceDirection};
use crate::error::VoiceError;

const PLAYBACK_LABELS: AudioThreadLabels = AudioThreadLabels {
    audio_thread: "audio-playback",
    error_bridge: "playback-error-bridge",
    play_failed: "failed to start output stream",
    spawn_failed: "failed to spawn playback thread",
    init_died: "playback thread died during init",
    direction: "playback",
};

/// Audio playback to the speaker via cpal.
///
/// Opens the system's default output device on a dedicated audio thread and
/// plays decoded PCM f32 chunks received from an mpsc channel. A `VecDeque`
/// ring buffer inside the audio callback smooths out timing differences.
///
/// The `cpal::Stream` lives entirely within the spawned thread (it is `!Send`
/// on macOS), so `AudioPlayback` itself is `Send`.
pub struct AudioPlayback {
    thread: AudioThread,
}

impl AudioPlayback {
    /// Create a new audio playback instance.
    pub fn new(sample_rate: u32, channels: u16) -> Result<Self, VoiceError> {
        Ok(Self {
            thread: AudioThread::new(sample_rate, channels, PLAYBACK_LABELS),
        })
    }

    /// Start playback, reading mixed PCM frames from the provided receiver.
    pub fn start(
        &mut self,
        rx: mpsc::Receiver<Vec<f32>>,
        device_name: Option<&str>,
        device_error_tx: Option<mpsc::Sender<String>>,
    ) -> Result<(), VoiceError> {
        self.thread.start(
            device_name,
            device_error_tx,
            move |sample_rate, channels, device_name_owned, error_tx| {
                build_playback_stream(
                    sample_rate,
                    channels,
                    rx,
                    device_name_owned.as_deref(),
                    error_tx,
                )
            },
        )
    }

    /// Stop playback.
    pub fn stop(&mut self) {
        self.thread.stop();
    }

    pub fn is_active(&self) -> bool {
        self.thread.is_active()
    }
}

/// Build a cpal output stream on the current thread.
fn build_playback_stream(
    sample_rate: u32,
    channels: u16,
    mut rx: mpsc::Receiver<Vec<f32>>,
    device_name: Option<&str>,
    error_tx: std_mpsc::Sender<String>,
) -> Result<cpal::Stream, VoiceError> {
    let host = cpal::default_host();
    let device = resolve_device(&host, device_name, &DeviceDirection::Output)?;

    let config = cpal::StreamConfig {
        channels,
        sample_rate: cpal::SampleRate(sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    // Pre-allocate the ring buffer — one second of audio is a generous ceiling
    let buffer_capacity = sample_rate as usize * usize::from(channels);
    let mut sample_buffer: VecDeque<f32> = VecDeque::with_capacity(buffer_capacity);

    device
        .build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                // Drain any available decoded audio from the channel
                while let Ok(samples) = rx.try_recv() {
                    sample_buffer.extend(samples);
                }
                // Fill the output buffer, substituting silence for missing samples
                for sample in data.iter_mut() {
                    *sample = sample_buffer.pop_front().unwrap_or(0.0);
                }
            },
            move |err: cpal::StreamError| {
                tracing::error!("output stream error: {err}");
                let _ = error_tx.send(format!("output: {err}"));
            },
            None,
        )
        .map_err(|e| VoiceError::AudioDevice(format!("failed to build output stream: {e}")))
}

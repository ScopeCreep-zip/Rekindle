use std::sync::mpsc as std_mpsc;

use cpal::traits::DeviceTrait;
use tokio::sync::mpsc;

use crate::audio_thread::{AudioThread, AudioThreadLabels};
use crate::device::{resolve_device, DeviceDirection};
use crate::error::VoiceError;
use crate::stream_config::{adapt_audio, negotiate_input_config};

const CAPTURE_LABELS: AudioThreadLabels = AudioThreadLabels {
    audio_thread: "audio-capture",
    error_bridge: "capture-error-bridge",
    play_failed: "failed to start input stream",
    spawn_failed: "failed to spawn capture thread",
    init_died: "capture thread died during init",
    direction: "capture",
};

/// Audio capture from the microphone via cpal.
///
/// Opens the system's default input device on a dedicated audio thread and
/// streams PCM f32 chunks to the provided mpsc sender. The `cpal::Stream`
/// lives entirely within the spawned thread (it is `!Send` on macOS), so
/// `AudioCapture` itself is `Send` and can be stored in shared state.
pub struct AudioCapture {
    thread: AudioThread,
}

impl AudioCapture {
    /// Create a new audio capture instance.
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            thread: AudioThread::new(sample_rate, channels, CAPTURE_LABELS),
        }
    }

    /// Start capturing audio, sending PCM frames to the provided sender.
    pub fn start(
        &mut self,
        tx: mpsc::Sender<Vec<f32>>,
        device_name: Option<&str>,
        device_error_tx: Option<mpsc::Sender<String>>,
    ) -> Result<(), VoiceError> {
        self.thread.start(
            device_name,
            device_error_tx,
            move |sample_rate, channels, device_name_owned, error_tx| {
                build_capture_stream(
                    sample_rate,
                    channels,
                    tx,
                    device_name_owned.as_deref(),
                    error_tx,
                )
            },
        )
    }

    /// Stop capturing audio.
    pub fn stop(&mut self) {
        self.thread.stop();
    }

    pub fn is_active(&self) -> bool {
        self.thread.is_active()
    }
}

/// Build a cpal input stream on the current thread.
///
/// `sample_rate`/`channels` are the pipeline (Opus) format. The device may
/// refuse that exact `StreamConfig`, so we negotiate a config it supports via
/// [`negotiate_input_config`] and, when it differs, adapt each captured buffer
/// back to the pipeline format with [`adapt_audio`] before forwarding.
fn build_capture_stream(
    sample_rate: u32,
    channels: u16,
    tx: mpsc::Sender<Vec<f32>>,
    device_name: Option<&str>,
    error_tx: std_mpsc::Sender<String>,
) -> Result<cpal::Stream, VoiceError> {
    let host = cpal::default_host();
    let device = resolve_device(&host, device_name, &DeviceDirection::Input)?;

    let (config, sample_format) = negotiate_input_config(&device, sample_rate, channels)?;
    let dev_channels = config.channels;
    let dev_rate = config.sample_rate.0;
    let needs_adapt = dev_channels != channels || dev_rate != sample_rate;

    tracing::info!(
        dev_channels,
        dev_rate,
        want_channels = channels,
        want_rate = sample_rate,
        ?sample_format,
        needs_adapt,
        "negotiated capture config"
    );

    // Forward captured f32 PCM to the pipeline, adapting to the codec format
    // first when the device config differs.
    let forward = move |samples: Vec<f32>, tx: &mpsc::Sender<Vec<f32>>| {
        let out = if needs_adapt {
            adapt_audio(&samples, dev_channels, dev_rate, channels, sample_rate)
        } else {
            samples
        };
        let _ = tx.try_send(out);
    };

    let make_error_callback = |error_tx: std_mpsc::Sender<String>| {
        move |err: cpal::StreamError| {
            tracing::error!("input stream error: {err}");
            let _ = error_tx.send(format!("input: {err}"));
        }
    };

    match sample_format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                forward(data.to_vec(), &tx);
            },
            make_error_callback(error_tx),
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                let samples: Vec<f32> = data
                    .iter()
                    .map(|&s| f32::from(s) / f32::from(i16::MAX))
                    .collect();
                forward(samples, &tx);
            },
            make_error_callback(error_tx),
            None,
        ),
        cpal::SampleFormat::U16 => device.build_input_stream(
            &config,
            move |data: &[u16], _: &cpal::InputCallbackInfo| {
                let samples: Vec<f32> = data
                    .iter()
                    .map(|&s| (f32::from(s) / f32::from(u16::MAX)) * 2.0 - 1.0)
                    .collect();
                forward(samples, &tx);
            },
            make_error_callback(error_tx),
            None,
        ),
        format => {
            return Err(VoiceError::AudioDevice(format!(
                "unsupported sample format: {format:?}"
            )))
        }
    }
    .map_err(|e| VoiceError::AudioDevice(format!("failed to build input stream: {e}")))
}

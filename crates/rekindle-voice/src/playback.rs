use std::collections::VecDeque;
use std::sync::mpsc as std_mpsc;

use cpal::traits::DeviceTrait;
use tokio::sync::mpsc;

use crate::audio_thread::{AudioThread, AudioThreadLabels};
use crate::device::{resolve_device, DeviceDirection};
use crate::error::VoiceError;
use crate::stream_config::{adapt_audio, negotiate_output_config};

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
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            thread: AudioThread::new(sample_rate, channels, PLAYBACK_LABELS),
        }
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
///
/// `sample_rate`/`channels` are the pipeline (mixer) format. The device may
/// refuse that exact `StreamConfig` and may also want a non-f32 sample format,
/// so we negotiate a supported config via [`negotiate_output_config`] and adapt
/// each drained buffer from the pipeline format to the device format before
/// filling the output, converting to the device's sample type on write.
fn build_playback_stream(
    sample_rate: u32,
    channels: u16,
    rx: mpsc::Receiver<Vec<f32>>,
    device_name: Option<&str>,
    error_tx: std_mpsc::Sender<String>,
) -> Result<cpal::Stream, VoiceError> {
    let host = cpal::default_host();
    let device = resolve_device(&host, device_name, &DeviceDirection::Output)?;

    let (config, sample_format) = negotiate_output_config(&device, sample_rate, channels)?;
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
        "negotiated playback config"
    );

    // Pre-allocate the ring buffer — one second of device audio is a generous
    // ceiling.
    let buffer_capacity = dev_rate as usize * usize::from(dev_channels.max(1));

    let error_callback = move |err: cpal::StreamError| {
        tracing::error!("output stream error: {err}");
        let _ = error_tx.send(format!("output: {err}"));
    };

    match sample_format {
        cpal::SampleFormat::F32 => device.build_output_stream(
            &config,
            output_callback::<f32>(
                rx,
                needs_adapt,
                channels,
                sample_rate,
                dev_channels,
                dev_rate,
                buffer_capacity,
            ),
            error_callback,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_output_stream(
            &config,
            output_callback::<i16>(
                rx,
                needs_adapt,
                channels,
                sample_rate,
                dev_channels,
                dev_rate,
                buffer_capacity,
            ),
            error_callback,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_output_stream(
            &config,
            output_callback::<u16>(
                rx,
                needs_adapt,
                channels,
                sample_rate,
                dev_channels,
                dev_rate,
                buffer_capacity,
            ),
            error_callback,
            None,
        ),
        format => {
            return Err(VoiceError::AudioDevice(format!(
                "unsupported sample format: {format:?}"
            )))
        }
    }
    .map_err(|e| VoiceError::AudioDevice(format!("failed to build output stream: {e}")))
}

/// Build the cpal output data callback for a device sample type `T`.
///
/// Drains decoded f32 PCM from `rx` (pipeline format), adapts it to the device
/// `(channels, rate)` when they differ, buffers it in a ring, and fills each
/// output slot — converting f32 → `T` on write and substituting silence when
/// the buffer underruns.
fn output_callback<T>(
    mut rx: mpsc::Receiver<Vec<f32>>,
    needs_adapt: bool,
    src_channels: u16,
    src_rate: u32,
    dst_channels: u16,
    dst_rate: u32,
    buffer_capacity: usize,
) -> impl FnMut(&mut [T], &cpal::OutputCallbackInfo)
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let mut sample_buffer: VecDeque<f32> = VecDeque::with_capacity(buffer_capacity);
    move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
        while let Ok(samples) = rx.try_recv() {
            if needs_adapt {
                sample_buffer.extend(adapt_audio(
                    &samples,
                    src_channels,
                    src_rate,
                    dst_channels,
                    dst_rate,
                ));
            } else {
                sample_buffer.extend(samples);
            }
        }
        for slot in data.iter_mut() {
            *slot = T::from_sample(sample_buffer.pop_front().unwrap_or(0.0));
        }
    }
}

use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use cpal::traits::DeviceTrait;
use rekindle_types::audio::VoiceSource;
use tokio::sync::mpsc;

use crate::audio_thread::{AudioThread, AudioThreadLabels};
use crate::device::{resolve_device, DeviceDirection};
use crate::error::VoiceError;
use crate::stream_config::{adapt_audio, adapt_capture, negotiate_input_config};

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
    source: VoiceSource,
    /// Under [`VoiceSource::Silence`], the pipeline's sender, held open
    /// and never written, so the send loop sees a live, silent source.
    silence: Option<mpsc::Sender<Vec<f32>>>,
    speech: Option<tokio::task::JoinHandle<()>>,
}

impl AudioCapture {
    /// Create a new audio capture instance for `source`.
    pub fn new(sample_rate: u32, channels: u16, source: VoiceSource) -> Self {
        Self {
            thread: AudioThread::new(sample_rate, channels, CAPTURE_LABELS),
            source,
            silence: None,
            speech: None,
        }
    }

    /// Start capturing audio, sending PCM frames to the provided sender.
    ///
    /// `picked` is the input-channel choice for this device (0-based
    /// indices, plan C7.24b); `None` averages every channel.
    pub fn start(
        &mut self,
        tx: mpsc::Sender<Vec<f32>>,
        device_name: Option<&str>,
        picked: Option<Vec<u16>>,
        device_error_tx: Option<mpsc::Sender<String>>,
    ) -> Result<(), VoiceError> {
        match self.source {
            VoiceSource::Silence => {
                self.silence = Some(tx);
                return Ok(());
            }
            VoiceSource::Speech => {
                let runtime = tokio::runtime::Handle::try_current()
                    .map_err(|error| VoiceError::Session(error.to_string()))?;
                let (rate, channels) = self.thread.format();
                self.speech = Some(runtime.spawn(play_speech(tx, rate, channels)));
                return Ok(());
            }
            VoiceSource::Mic => {}
        }
        self.thread.start(
            device_name,
            device_error_tx,
            move |sample_rate, channels, device_name_owned, error_tx| {
                build_capture_stream(
                    sample_rate,
                    channels,
                    tx,
                    device_name_owned.as_deref(),
                    picked,
                    error_tx,
                )
            },
        )
    }

    /// Stop capturing audio.
    pub fn stop(&mut self) {
        if let Some(task) = self.speech.take() {
            task.abort();
            return;
        }
        if self.silence.take().is_none() && self.thread.is_active() {
            self.thread.stop();
        }
    }

    pub fn is_active(&self) -> bool {
        self.silence.is_some() || self.speech.is_some() || self.thread.is_active()
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

// Like a microphone callback, pace the PCM and drop frames under backpressure.
async fn play_speech(tx: mpsc::Sender<Vec<f32>>, rate: u32, channels: u16) {
    const SOURCE_RATE: u32 = 16_000;
    const SAMPLES_PER_FRAME: usize = 320;
    const SPEECH: &[u8] = include_bytes!("../assets/speech.pcm");
    let mut samples = SPEECH.chunks_exact(2).cycle();
    let mut frame = [0.0; SAMPLES_PER_FRAME];
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        for (out, bytes) in frame.iter_mut().zip(samples.by_ref()) {
            *out = f32::from(i16::from_le_bytes([bytes[0], bytes[1]])) / f32::from(i16::MAX);
        }
        let output = adapt_audio(&frame, 1, SOURCE_RATE, channels, rate);
        if let Err(mpsc::error::TrySendError::Closed(_)) = tx.try_send(output) {
            break;
        }
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
    picked: Option<Vec<u16>>,
    error_tx: std_mpsc::Sender<String>,
) -> Result<cpal::Stream, VoiceError> {
    let host = cpal::default_host();
    let device = resolve_device(&host, device_name, &DeviceDirection::Input)?;

    // A channel choice needs every input channel delivered, so open the
    // device at its full channel count rather than the pipeline's mono.
    let open_channels = if picked.is_some() {
        crate::device::max_input_channels(&device)
    } else {
        channels
    };
    let (config, sample_format) = negotiate_input_config(&device, sample_rate, open_channels)?;
    let dev_channels = config.channels;
    let dev_rate = config.sample_rate;
    if let Some(bad) = picked.iter().flatten().find(|&&c| c >= dev_channels) {
        return Err(VoiceError::AudioDevice(format!(
            "input channel {} is not on this device ({dev_channels} channels)",
            bad + 1
        )));
    }
    let needs_adapt = picked.is_some() || dev_channels != channels || dev_rate != sample_rate;

    tracing::info!(
        device = %crate::device::device_label(&device),
        dev_channels,
        dev_rate,
        want_channels = channels,
        want_rate = sample_rate,
        ?sample_format,
        needs_adapt,
        ?picked,
        "negotiated capture config"
    );

    // Forward captured f32 PCM to the pipeline, adapting to the codec format
    // first when the device config differs.
    let forward = move |samples: Vec<f32>, tx: &mpsc::Sender<Vec<f32>>| {
        let out = if let Some(picked) = &picked {
            adapt_capture(
                &samples,
                dev_channels,
                picked,
                dev_rate,
                channels,
                sample_rate,
            )
        } else if needs_adapt {
            adapt_audio(&samples, dev_channels, dev_rate, channels, sample_rate)
        } else {
            samples
        };
        let _ = tx.try_send(out);
    };

    let make_error_callback = |error_tx: std_mpsc::Sender<String>| {
        move |err: cpal::Error| crate::device::on_stream_error("input", &err, &error_tx)
    };

    match sample_format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                forward(data.to_vec(), &tx);
            },
            make_error_callback(error_tx),
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            config,
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
            config,
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

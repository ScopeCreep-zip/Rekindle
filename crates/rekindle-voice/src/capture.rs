use std::sync::mpsc as std_mpsc;
use std::thread;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tokio::sync::mpsc;

use crate::error::VoiceError;

/// Audio capture from the microphone via cpal.
///
/// Opens the system's default input device on a dedicated audio thread and
/// streams PCM f32 chunks to the provided mpsc sender. The `cpal::Stream`
/// lives entirely within the spawned thread (it is `!Send` on macOS), so
/// `AudioCapture` itself is `Send` and can be stored in shared state.
pub struct AudioCapture {
    is_active: bool,
    sample_rate: u32,
    channels: u16,
    /// Dropping this sender signals the audio thread to shut down.
    shutdown_tx: Option<std_mpsc::Sender<()>>,
    /// Handle to the dedicated audio thread.
    thread_handle: Option<thread::JoinHandle<()>>,
}

impl AudioCapture {
    /// Create a new audio capture instance.
    pub fn new(sample_rate: u32, channels: u16) -> Result<Self, VoiceError> {
        Ok(Self {
            is_active: false,
            sample_rate,
            channels,
            shutdown_tx: None,
            thread_handle: None,
        })
    }

    /// Start capturing audio, sending PCM frames to the provided sender.
    ///
    /// Spawns a dedicated thread that owns the cpal stream. The thread blocks
    /// on a shutdown channel until `stop()` is called. Any errors during
    /// device/stream initialisation are propagated back via a sync channel.
    pub fn start(&mut self, tx: mpsc::Sender<Vec<f32>>) -> Result<(), VoiceError> {
        let (init_tx, init_rx) = std_mpsc::sync_channel::<Result<(), VoiceError>>(1);
        let (shutdown_tx, shutdown_rx) = std_mpsc::channel::<()>();

        let sample_rate = self.sample_rate;
        let channels = self.channels;

        let handle = thread::Builder::new()
            .name("audio-capture".into())
            .spawn(move || {
                let result = build_capture_stream(sample_rate, channels, tx);
                match result {
                    Ok(stream) => {
                        if let Err(e) = stream.play() {
                            let _ = init_tx.send(Err(VoiceError::AudioDevice(format!(
                                "failed to start input stream: {e}"
                            ))));
                            return;
                        }
                        let _ = init_tx.send(Ok(()));
                        // Park until shutdown — stream stays alive in this scope
                        let _ = shutdown_rx.recv();
                        drop(stream);
                    }
                    Err(e) => {
                        let _ = init_tx.send(Err(e));
                    }
                }
            })
            .map_err(|e| {
                VoiceError::AudioDevice(format!("failed to spawn capture thread: {e}"))
            })?;

        // Wait for the audio thread to report success or failure
        init_rx
            .recv()
            .map_err(|_| VoiceError::AudioDevice("capture thread died during init".into()))??;

        self.shutdown_tx = Some(shutdown_tx);
        self.thread_handle = Some(handle);
        self.is_active = true;
        tracing::info!(
            sample_rate = self.sample_rate,
            channels = self.channels,
            "audio capture started"
        );
        Ok(())
    }

    /// Stop capturing audio. Signals the audio thread to shut down and waits
    /// for it to exit so the device is cleanly released.
    pub fn stop(&mut self) {
        // Dropping the sender causes recv() in the thread to return Err,
        // which exits the thread and drops the cpal stream.
        self.shutdown_tx = None;
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
        self.is_active = false;
        tracing::info!("audio capture stopped");
    }

    pub fn is_active(&self) -> bool {
        self.is_active
    }
}

/// Build a cpal input stream on the current thread. The returned `Stream`
/// must be kept alive for audio to flow — dropping it stops the capture.
fn build_capture_stream(
    sample_rate: u32,
    channels: u16,
    tx: mpsc::Sender<Vec<f32>>,
) -> Result<cpal::Stream, VoiceError> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| VoiceError::AudioDevice("no input device available".into()))?;

    let supported = device
        .default_input_config()
        .map_err(|e| VoiceError::AudioDevice(format!("no input config: {e}")))?;

    let sample_format = supported.sample_format();
    let config = cpal::StreamConfig {
        channels,
        sample_rate: cpal::SampleRate(sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    match sample_format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                let _ = tx.try_send(data.to_vec());
            },
            |err| tracing::error!("input stream error: {err}"),
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                let samples: Vec<f32> =
                    data.iter().map(|&s| f32::from(s) / f32::from(i16::MAX)).collect();
                let _ = tx.try_send(samples);
            },
            |err| tracing::error!("input stream error: {err}"),
            None,
        ),
        cpal::SampleFormat::U16 => device.build_input_stream(
            &config,
            move |data: &[u16], _: &cpal::InputCallbackInfo| {
                let samples: Vec<f32> = data
                    .iter()
                    .map(|&s| (f32::from(s) / f32::from(u16::MAX)) * 2.0 - 1.0)
                    .collect();
                let _ = tx.try_send(samples);
            },
            |err| tracing::error!("input stream error: {err}"),
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

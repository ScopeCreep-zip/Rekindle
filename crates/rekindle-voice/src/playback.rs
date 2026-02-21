use std::collections::VecDeque;
use std::sync::mpsc as std_mpsc;
use std::thread;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tokio::sync::mpsc;

use crate::error::VoiceError;

/// Audio playback to the speaker via cpal.
///
/// Opens the system's default output device on a dedicated audio thread and
/// plays decoded PCM f32 chunks received from an mpsc channel. A `VecDeque`
/// ring buffer inside the audio callback smooths out timing differences.
///
/// The `cpal::Stream` lives entirely within the spawned thread (it is `!Send`
/// on macOS), so `AudioPlayback` itself is `Send`.
pub struct AudioPlayback {
    is_active: bool,
    sample_rate: u32,
    channels: u16,
    /// Dropping this sender signals the audio thread to shut down.
    shutdown_tx: Option<std_mpsc::Sender<()>>,
    /// Handle to the dedicated audio thread.
    thread_handle: Option<thread::JoinHandle<()>>,
    /// Handle to the error bridge thread (forwards cpal errors to async channel).
    error_bridge_handle: Option<thread::JoinHandle<()>>,
}

impl AudioPlayback {
    /// Create a new audio playback instance.
    pub fn new(sample_rate: u32, channels: u16) -> Result<Self, VoiceError> {
        Ok(Self {
            is_active: false,
            sample_rate,
            channels,
            shutdown_tx: None,
            thread_handle: None,
            error_bridge_handle: None,
        })
    }

    /// Start playback, reading mixed PCM frames from the provided receiver.
    ///
    /// Spawns a dedicated thread that owns the cpal output stream. The receiver
    /// is moved into the audio callback where `try_recv` drains decoded chunks
    /// into a `VecDeque`. When no data is available, silence (0.0) is output.
    ///
    /// `device_name`: optional device name to use. `None` = system default.
    /// `device_error_tx`: optional channel to signal device errors (e.g. device unplugged).
    pub fn start(
        &mut self,
        rx: mpsc::Receiver<Vec<f32>>,
        device_name: Option<&str>,
        device_error_tx: Option<mpsc::Sender<String>>,
    ) -> Result<(), VoiceError> {
        let (init_tx, init_rx) = std_mpsc::sync_channel::<Result<(), VoiceError>>(1);
        let (shutdown_tx, shutdown_rx) = std_mpsc::channel::<()>();

        let sample_rate = self.sample_rate;
        let channels = self.channels;
        let device_name_owned = device_name.map(String::from);

        // Bridge sync error callback → async error channel
        let (sync_err_tx, sync_err_rx) = std_mpsc::channel::<String>();
        let error_bridge_handle = if let Some(async_err_tx) = device_error_tx {
            thread::Builder::new()
                .name("playback-error-bridge".into())
                .spawn(move || {
                    if let Ok(err_msg) = sync_err_rx.recv() {
                        let _ = async_err_tx.blocking_send(err_msg);
                    }
                })
                .ok()
        } else {
            None
        };

        let handle = thread::Builder::new()
            .name("audio-playback".into())
            .spawn(move || {
                let result = build_playback_stream(
                    sample_rate,
                    channels,
                    rx,
                    device_name_owned.as_deref(),
                    sync_err_tx,
                );
                match result {
                    Ok(stream) => {
                        if let Err(e) = stream.play() {
                            let _ = init_tx.send(Err(VoiceError::AudioDevice(format!(
                                "failed to start output stream: {e}"
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
                VoiceError::AudioDevice(format!("failed to spawn playback thread: {e}"))
            })?;

        // Wait for the audio thread to report success or failure
        init_rx
            .recv()
            .map_err(|_| VoiceError::AudioDevice("playback thread died during init".into()))??;

        self.shutdown_tx = Some(shutdown_tx);
        self.thread_handle = Some(handle);
        self.error_bridge_handle = error_bridge_handle;
        self.is_active = true;
        tracing::info!(
            sample_rate = self.sample_rate,
            channels = self.channels,
            "audio playback started"
        );
        Ok(())
    }

    /// Stop playback. Signals the audio thread to shut down and waits
    /// for it to exit so the device is cleanly released.
    pub fn stop(&mut self) {
        self.shutdown_tx = None;
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
        // Join the error bridge thread (it will exit once sync_err_tx is dropped
        // by the audio thread, causing recv() to return Err).
        if let Some(handle) = self.error_bridge_handle.take() {
            let _ = handle.join();
        }
        self.is_active = false;
        tracing::info!("audio playback stopped");
    }

    pub fn is_active(&self) -> bool {
        self.is_active
    }
}

impl Drop for AudioPlayback {
    fn drop(&mut self) {
        if self.is_active {
            self.stop();
        }
    }
}

/// Build a cpal output stream on the current thread. The `rx` receiver is
/// moved into the output callback and drained via `try_recv` each tick.
fn build_playback_stream(
    sample_rate: u32,
    channels: u16,
    mut rx: mpsc::Receiver<Vec<f32>>,
    device_name: Option<&str>,
    error_tx: std_mpsc::Sender<String>,
) -> Result<cpal::Stream, VoiceError> {
    let host = cpal::default_host();
    let device = match device_name {
        Some(name) => find_output_device(&host, name)?,
        None => host
            .default_output_device()
            .ok_or_else(|| VoiceError::AudioDevice("no output device available".into()))?,
    };

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

/// Find an output device by name, falling back to the default if not found.
fn find_output_device(host: &cpal::Host, name: &str) -> Result<cpal::Device, VoiceError> {
    crate::device::find_device(host, name, &crate::device::DeviceDirection::Output)
}

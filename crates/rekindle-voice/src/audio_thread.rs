use std::sync::mpsc as std_mpsc;
use std::thread;

use cpal::traits::StreamTrait;
use tokio::sync::mpsc;

use crate::error::VoiceError;

/// Labels for thread names, error messages, and log output.
///
/// Using `&'static str` since all values are compile-time literals.
pub(crate) struct AudioThreadLabels {
    /// Thread name for the audio thread (e.g. "audio-capture").
    pub audio_thread: &'static str,
    /// Thread name for the error bridge thread (e.g. "capture-error-bridge").
    pub error_bridge: &'static str,
    /// Error message prefix for `stream.play()` failure.
    pub play_failed: &'static str,
    /// Error message for thread spawn failure.
    pub spawn_failed: &'static str,
    /// Error message for thread death during init.
    pub init_died: &'static str,
    /// Direction label for structured tracing (e.g. "capture").
    pub direction: &'static str,
}

/// Manages the lifecycle of a dedicated audio thread.
///
/// `cpal::Stream` is `!Send` on macOS, so the stream must live on a dedicated
/// OS thread. This struct owns the thread handles and shutdown channel,
/// providing a generic `start()` that accepts a stream-builder closure.
pub(crate) struct AudioThread {
    is_active: bool,
    sample_rate: u32,
    channels: u16,
    shutdown_tx: Option<std_mpsc::Sender<()>>,
    thread_handle: Option<thread::JoinHandle<()>>,
    error_bridge_handle: Option<thread::JoinHandle<()>>,
    labels: AudioThreadLabels,
}

impl AudioThread {
    pub(crate) fn new(sample_rate: u32, channels: u16, labels: AudioThreadLabels) -> Self {
        Self {
            is_active: false,
            sample_rate,
            channels,
            shutdown_tx: None,
            thread_handle: None,
            error_bridge_handle: None,
            labels,
        }
    }

    /// Start the audio thread.
    ///
    /// `build_stream` runs on the dedicated thread and must return a `cpal::Stream`.
    /// It receives `(sample_rate, channels, device_name, error_tx)` — the closure
    /// captures direction-specific data (e.g. the pcm sender or receiver).
    pub(crate) fn start<F>(
        &mut self,
        device_name: Option<&str>,
        device_error_tx: Option<mpsc::Sender<String>>,
        build_stream: F,
    ) -> Result<(), VoiceError>
    where
        F: FnOnce(
                u32,
                u16,
                Option<String>,
                std_mpsc::Sender<String>,
            ) -> Result<cpal::Stream, VoiceError>
            + Send
            + 'static,
    {
        let (init_tx, init_rx) = std_mpsc::sync_channel::<Result<(), VoiceError>>(1);
        let (shutdown_tx, shutdown_rx) = std_mpsc::channel::<()>();

        let sample_rate = self.sample_rate;
        let channels = self.channels;
        let device_name_owned = device_name.map(String::from);

        // Bridge sync error callback → async error channel.
        // cpal's error callback runs on the audio thread (sync), so we use a
        // std channel to receive the error and a bridging thread to forward it.
        let (sync_err_tx, sync_err_rx) = std_mpsc::channel::<String>();
        let error_bridge_handle = if let Some(async_err_tx) = device_error_tx {
            thread::Builder::new()
                .name(self.labels.error_bridge.into())
                .spawn(move || {
                    if let Ok(err_msg) = sync_err_rx.recv() {
                        let _ = async_err_tx.blocking_send(err_msg);
                    }
                })
                .ok()
        } else {
            None
        };

        let play_failed = self.labels.play_failed;
        let spawn_failed = self.labels.spawn_failed;
        let handle = thread::Builder::new()
            .name(self.labels.audio_thread.into())
            .spawn(move || {
                let result = build_stream(sample_rate, channels, device_name_owned, sync_err_tx);
                match result {
                    Ok(stream) => {
                        if let Err(e) = stream.play() {
                            let _ = init_tx
                                .send(Err(VoiceError::AudioDevice(format!("{play_failed}: {e}"))));
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
            .map_err(|e| VoiceError::AudioDevice(format!("{spawn_failed}: {e}")))?;

        // Wait for the audio thread to report success or failure
        init_rx
            .recv()
            .map_err(|_| VoiceError::AudioDevice(self.labels.init_died.into()))??;

        self.shutdown_tx = Some(shutdown_tx);
        self.thread_handle = Some(handle);
        self.error_bridge_handle = error_bridge_handle;
        self.is_active = true;
        tracing::info!(
            direction = self.labels.direction,
            sample_rate = self.sample_rate,
            channels = self.channels,
            "audio thread started"
        );
        Ok(())
    }

    /// Signal the audio thread to shut down and wait for it to exit.
    pub(crate) fn stop(&mut self) {
        // Dropping the sender causes recv() in the thread to return Err,
        // which exits the thread and drops the cpal stream.
        self.shutdown_tx = None;
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.error_bridge_handle.take() {
            let _ = handle.join();
        }
        self.is_active = false;
        tracing::info!(direction = self.labels.direction, "audio thread stopped");
    }

    pub(crate) fn is_active(&self) -> bool {
        self.is_active
    }
}

impl Drop for AudioThread {
    fn drop(&mut self) {
        if self.is_active {
            self.stop();
        }
    }
}

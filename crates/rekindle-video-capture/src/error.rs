use thiserror::Error;

#[derive(Debug, Error)]
pub enum CaptureError {
    /// GStreamer init failed or a required element/property is absent
    /// — `capture_available()` returns false for the same conditions,
    /// so a session start hitting this means the caller skipped the
    /// availability gate.
    #[error("capture stack unavailable: {0}")]
    Unavailable(String),
    /// The camera is owned by another streaming consumer. V4L2 grants
    /// streaming ownership per-fd: the loser fails at S_FMT with EBUSY
    /// ~tens of ms AFTER the pipeline reaches PLAYING (async bus
    /// error), which is why session start waits for the first sample.
    #[error("camera busy: {0}")]
    Busy(String),
    /// Device missing/unreadable (unplugged, permissions).
    #[error("camera unavailable: {0}")]
    Device(String),
    /// Pipeline reached PLAYING but produced no sample inside the
    /// start deadline and no bus error explained why.
    #[error("camera timeout: no frames from {0}")]
    Timeout(String),
    /// Any other GStreamer failure (state change, property, bus).
    #[error("pipeline error: {0}")]
    Pipeline(String),
}

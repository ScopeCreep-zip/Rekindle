use thiserror::Error;

#[derive(Debug, Error)]
pub enum VoiceError {
    #[error("audio device error: {0}")]
    AudioDevice(String),

    #[error("codec error: {0}")]
    Codec(String),

    #[error("transport error: {0}")]
    Transport(String),

    #[error("not connected to voice channel")]
    NotConnected,
}

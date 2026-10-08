use thiserror::Error;

#[derive(Debug, Error)]
pub enum VoiceError {
    /// The community's Lamport clock could not produce a timestamp.
    #[error(transparent)]
    Lamport(#[from] rekindle_types::lamport::LamportError),

    #[error("audio device error: {0}")]
    AudioDevice(String),

    #[error("codec error: {0}")]
    Codec(String),

    #[error("transport error: {0}")]
    Transport(String),

    #[error("not connected to voice channel")]
    NotConnected,

    // Phase 14 additions for the deps trait surface.
    #[error("identity not loaded")]
    IdentityNotLoaded,

    #[error("session: {0}")]
    Session(String),

    /// We hold no media-class route, so no peer could send us media. A
    /// join fails with this rather than advertise the general route
    /// (plan C7.9c: no fallback); the window shows the media route's state.
    #[error("media route unavailable")]
    MediaRouteUnavailable,
}

//! Host audio endpoint selection and notices for audio device outcomes.

/// Where the voice engine gets its outbound PCM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceSource {
    Mic,
    /// Keep the capture pipeline live without opening a device or sending PCM.
    Silence,
    /// Loop a recorded speech clip through the capture pipeline.
    Speech,
}

/// Where the voice engine sends decoded audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceSink {
    Speakers,
    /// Discard decoded PCM without opening a device.
    Null,
}

/// The audio endpoints selected by the Host for a voice session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioIo {
    pub source: VoiceSource,
    pub sink: VoiceSink,
}

impl AudioIo {
    pub const DEVICES: Self = Self {
        source: VoiceSource::Mic,
        sink: VoiceSink::Speakers,
    };

    pub const HEADLESS: Self = Self {
        source: VoiceSource::Silence,
        sink: VoiceSink::Null,
    };
}

/// The end of the audio pipeline a device serves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioDeviceKind {
    Input,
    Output,
}

impl AudioDeviceKind {
    /// The code in a device-change event.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Output => "output",
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Input => "microphone",
            Self::Output => "speaker",
        }
    }
}

/// Why an audio device changed while voice was running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioDeviceChange {
    SavedDisconnected,
    SavedReconnected,
    DefaultChanged,
}

impl AudioDeviceChange {
    /// The reason sent with the device-change event.
    #[must_use]
    pub const fn notice(self) -> &'static str {
        match self {
            Self::SavedDisconnected => "saved device disconnected",
            Self::SavedReconnected => "saved device reconnected",
            Self::DefaultChanged => "system default changed",
        }
    }
}

/// A user-facing explanation of an audio device event.
pub enum AudioNotice<'a> {
    SavedDeviceMissing {
        kind: AudioDeviceKind,
        selected: &'a str,
    },
    DeviceSwitched {
        kind: AudioDeviceKind,
        selected: &'a str,
        change: AudioDeviceChange,
    },
    RestartFailed,
}

impl AudioNotice<'_> {
    /// The alert title and body.
    #[must_use]
    pub fn notice(self) -> (String, String) {
        match self {
            Self::SavedDeviceMissing { kind, selected } => {
                let device = kind.name();
                (
                    format!("Your {device} isn't connected"),
                    format!(
                        "Your chosen {device} isn't connected, so Rekindle is using \"{selected}\". It switches back when it is plugged in."
                    ),
                )
            }
            Self::DeviceSwitched {
                kind,
                selected,
                change,
            } => (
                "Audio device switched".into(),
                format!(
                    "Now using \"{selected}\" for {} ({}).",
                    kind.code(),
                    change.notice()
                ),
            ),
            Self::RestartFailed => (
                "Audio couldn't restart".into(),
                "Rekindle couldn't reopen your audio devices. Leave and rejoin the call.".into(),
            ),
        }
    }
}

/// User-facing failure at the voice-session command boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceFailure {
    NoDevice(AudioDeviceKind),
    AudioDevice,
    SignInRequired,
    NotConnected,
    MediaRouteUnavailable,
    ChannelUnavailable,
    Session,
}

impl VoiceFailure {
    /// The message to show. Device-driver and transport details belong in logs.
    #[must_use]
    pub fn notice(self) -> String {
        match self {
            Self::NoDevice(kind) => format!("No {} is available.", kind.name()),
            Self::AudioDevice => "Rekindle couldn't open your audio devices.".into(),
            Self::SignInRequired => "Sign in to use voice.".into(),
            Self::NotConnected => "You're not connected to a voice channel.".into(),
            Self::MediaRouteUnavailable => "Your media route is unavailable.".into(),
            Self::ChannelUnavailable => "That voice channel is unavailable.".into(),
            Self::Session => "Rekindle couldn't complete the voice action.".into(),
        }
    }
}

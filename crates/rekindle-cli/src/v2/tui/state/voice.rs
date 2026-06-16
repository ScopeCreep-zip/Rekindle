//! Voice session state.

#[derive(Clone, Debug)]
pub struct ActiveVoiceSession {
    pub community: String,
    pub channel: String,
    pub muted: bool,
    pub deafened: bool,
    pub participants: Vec<VoiceParticipant>,
}

#[derive(Clone, Debug)]
pub struct VoiceParticipant {
    pub pseudonym: String,
    pub muted: bool,
    pub deafened: bool,
}

#[derive(Debug, Default)]
pub struct VoiceState {
    pub active_session: Option<ActiveVoiceSession>,
}

impl VoiceState {
    pub fn new() -> Self {
        Self::default()
    }
}

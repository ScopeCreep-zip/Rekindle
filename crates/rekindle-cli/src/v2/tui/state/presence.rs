//! Presence state — cleared on reconnect.

#[derive(Debug, Default)]
pub struct PresenceState;

impl PresenceState {
    pub fn new() -> Self {
        Self
    }
}

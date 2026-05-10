//! Voice delegation — join, leave, mute, deafen.

use crate::ChatError;
use super::super::ChatService;

impl ChatService {
    pub async fn voice_join(
        &self,
        community: &str,
        channel: &str,
        muted: bool,
        deafened: bool,
    ) -> Result<(), ChatError> {
        self.voice.join_voice(community, channel, muted, deafened).await
    }

    pub async fn voice_leave(&self) -> Result<(), ChatError> {
        self.voice.leave_voice().await
    }

    pub async fn voice_mute(&self, muted: bool) -> Result<(), ChatError> {
        self.voice.set_mute(muted).await
    }

    pub async fn voice_deafen(&self, deafened: bool) -> Result<(), ChatError> {
        self.voice.set_deafen(deafened).await
    }
}

//! The soundboard and the voice stage's hand raises over the Host's
//! channel adapter. Their requests are plan E4.1's (`PENDING_PORT` rows
//! `upload_soundboard_sound`, `play_soundboard`); until then the shell's
//! commands and its voice signaling call these.

use rekindle_channel::error::ChannelError;

use super::HostChannel;

impl HostChannel {
    /// Upload a soundboard sound; its expression id.
    ///
    /// # Errors
    /// We may not manage expressions, or the upload or entry failed.
    pub async fn upload_soundboard_sound(
        &self,
        params: rekindle_channel::UploadSoundboardSoundParams<'_>,
    ) -> Result<String, ChannelError> {
        rekindle_channel::upload_soundboard_sound(self, params).await
    }

    /// Play the soundboard sound `expression_id_hex` in `channel_id`.
    ///
    /// # Errors
    /// The sound is unknown, or the mesh send failed.
    pub fn play_soundboard(
        &self,
        community_id: &str,
        channel_id: &str,
        expression_id_hex: &str,
    ) -> Result<(), ChannelError> {
        rekindle_channel::play_soundboard(self, community_id, channel_id, expression_id_hex)
    }

    /// Raise or lower our hand on the stage `channel_id`.
    ///
    /// # Errors
    /// The record write failed.
    pub async fn persist_hand_raise(
        &self,
        community_id: &str,
        channel_id: &str,
        raised: bool,
    ) -> Result<(), ChannelError> {
        rekindle_channel::persist_hand_raise(self, community_id, channel_id, raised).await
    }

    /// The pseudonyms with a hand raised on the stage `channel_id`.
    ///
    /// # Errors
    /// The record read failed.
    pub async fn hand_raises(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Result<Vec<String>, ChannelError> {
        rekindle_channel::list_hand_raises(self, community_id, channel_id).await
    }

    /// Whether a message's cleartext mentions reach us, reader-validated:
    /// `@everyone` and `@here` count only when the sender holds
    /// `MENTION_EVERYONE` in the merged governance state.
    #[must_use]
    pub fn mentions_us(&self, community_id: &str, mention: &MentionFields<'_>) -> bool {
        let mut matches = rekindle_channel::matches_from_cleartext(
            self,
            community_id,
            mention.pseudonyms,
            mention.roles,
            mention.everyone,
            mention.here,
        );
        rekindle_channel::validate_sender_permissions(
            self,
            community_id,
            mention.sender_pseudonym,
            &mut matches,
        );
        rekindle_channel::local_member_is_mentioned(self, community_id, &matches)
    }
}

/// A received message's cleartext mention fields.
pub struct MentionFields<'a> {
    pub sender_pseudonym: &'a str,
    pub pseudonyms: &'a [String],
    pub roles: &'a [String],
    pub everyone: bool,
    pub here: bool,
}

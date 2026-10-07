//! The sealing class of each 1:1 payload: whether it travels encrypted
//! with the Signal session or signed only.

use super::payload::MessagePayload;

/// How a payload must travel inside its signed envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sealing {
    /// Signed but not encrypted. Payloads that can precede a Signal
    /// session (friend handshake, session reset) or have no session-layer
    /// carrier yet (call signaling, relay, push and status probes).
    Plain,
    /// Encrypted with the Signal session for the recipient.
    Session,
}

impl MessagePayload {
    /// The sealing every sender must use and every receiver demands for
    /// this payload. Exhaustive on purpose: a new variant must choose.
    /// The `Plain` set shrinks when call signaling moves onto the session
    /// (plan step E4.1) and is empty once `MessageEnvelope` is deleted
    /// (plan step E2.3).
    #[must_use]
    pub fn sealing(&self) -> Sealing {
        match self {
            Self::DirectMessage { .. }
            | Self::ChannelMessage { .. }
            | Self::TypingIndicator { .. }
            | Self::PresenceUpdate { .. }
            | Self::GroupDmInvite { .. }
            | Self::DmLeave { .. }
            | Self::DmVideoFragment { .. } => Sealing::Session,
            Self::FriendRequest { .. }
            | Self::FriendAccept { .. }
            | Self::FriendReject
            | Self::ProfileKeyRotated { .. }
            | Self::FriendRequestReceived
            | Self::Unfriended
            | Self::UnfriendedAck
            | Self::RelayOffer { .. }
            | Self::RelayWithdraw { .. }
            | Self::RelayOfferAck { .. }
            | Self::RelayEnvelope { .. }
            | Self::DmInvite { .. }
            | Self::DmAccept { .. }
            | Self::DmDecline { .. }
            | Self::RegisterPushRelay { .. }
            | Self::UnregisterPushRelay { .. }
            | Self::WakeNotify { .. }
            | Self::StatusRequest { .. }
            | Self::StatusResponse { .. }
            | Self::CallInvite { .. }
            | Self::CallRinging { .. }
            | Self::CallAccept { .. }
            | Self::CallDecline { .. }
            | Self::CallEnd { .. }
            | Self::CallMediaState { .. }
            | Self::CallReaction { .. }
            | Self::GroupCallOffer { .. }
            | Self::GroupCallAccept { .. }
            | Self::GroupCallDecline { .. }
            | Self::GroupCallParticipantJoined { .. }
            | Self::GroupCallParticipantLeft { .. }
            | Self::SessionResetRequest { .. }
            | Self::SessionResetAccept { .. }
            | Self::SessionResetDecline { .. } => Sealing::Plain,
        }
    }
}

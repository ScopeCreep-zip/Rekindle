//! Video session facts (plan E4.1): what the backend tells the window that
//! captures, encodes or renders a scope's video. Each names its
//! [`MediaScope`], so a window keyed by `(WindowLabel, MediaScope)` takes
//! only its own. Encoder targets and keyframe requests come down here;
//! encoded frames go up on the media lane (`BusPayload::MediaUp`).

use serde::{Deserialize, Serialize};

use crate::media::MediaScope;
use crate::video::SessionVideoConfig;

/// A peer whose advertised decoder set blocks codec negotiation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoCodecPeer {
    /// Identity key or pseudonym hex, according to the event's scope.
    pub sender: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum VideoEvent {
    /// A receiver asked for a keyframe of our `stream_id` (RFC 5104 FIR).
    KeyframeRequest {
        scope: MediaScope,
        /// Who asked (identity key or pseudonym hex).
        sender: String,
        stream_id: String,
    },
    /// The scope's negotiated encoder and decoder constraints.
    SessionConfig {
        scope: MediaScope,
        config: SessionVideoConfig,
    },
    /// These peers share no codec with us.
    CodecIncompatible {
        scope: MediaScope,
        peers: Vec<VideoCodecPeer>,
    },
    /// The bandwidth owner's encoder target for the scope.
    BitrateTarget { scope: MediaScope, kbps: u32 },
    /// A video envelope from `sender` failed verification at the receive
    /// boundary.
    EnvelopeRejected {
        scope: MediaScope,
        sender: String,
        display_name: String,
        reason: String,
    },
    /// `sender`'s `stream_id` now reaches us through `relay_host` (or
    /// directly when `None`).
    TopologyChange {
        scope: MediaScope,
        sender: String,
        stream_id: String,
        relay_host: Option<String>,
        reason: String,
        lamport: u64,
    },
}

impl VideoEvent {
    /// D21 permits shedding these frame-rate hints on a full delivery queue.
    /// The journal assigns a cursor before delivery applies this policy.
    #[must_use]
    pub const fn can_shed(&self) -> bool {
        matches!(
            self,
            Self::KeyframeRequest { .. } | Self::BitrateTarget { .. }
        )
    }

    /// The scope this event is about.
    #[must_use]
    pub fn scope(&self) -> &MediaScope {
        match self {
            Self::KeyframeRequest { scope, .. }
            | Self::SessionConfig { scope, .. }
            | Self::CodecIncompatible { scope, .. }
            | Self::BitrateTarget { scope, .. }
            | Self::EnvelopeRejected { scope, .. }
            | Self::TopologyChange { scope, .. } => scope,
        }
    }

    /// The community of a channel scope; `None` for a direct call.
    #[must_use]
    pub fn community(&self) -> Option<&str> {
        match self.scope() {
            MediaScope::Channel { community_id, .. } => Some(community_id),
            MediaScope::DirectCall { .. } => None,
        }
    }
}

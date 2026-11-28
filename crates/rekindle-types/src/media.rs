//! The one media scope and track vocabulary (plan E4.1): the key of the
//! voice session's media keys, the encoded-media uplink and downlink, the
//! video channel registry and every `VideoEvent`. E4.2 adds
//! `GroupCall { call_id }` to this one type.
//!
//! Externally tagged and `camelCase`, like `SubscriptionEvent`: it crosses
//! postcard on the IPC bus (which cannot decode an internally tagged enum)
//! and JSON to the webview.

use serde::{Deserialize, Serialize};

/// What a media session is keyed under.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum MediaScope {
    /// A 1:1 call with `peer` (identity key hex).
    DirectCall { peer: String },
    /// A community voice channel.
    Channel {
        community_id: String,
        channel_id: String,
    },
}

impl MediaScope {
    /// The scope of a voice session on `channel_id`: a community channel
    /// when `community_id` is set, otherwise a 1:1 call whose channel id is
    /// the peer's identity key.
    #[must_use]
    pub fn of_session(community_id: Option<&str>, channel_id: &str) -> Self {
        match community_id {
            Some(community_id) => Self::Channel {
                community_id: community_id.to_string(),
                channel_id: channel_id.to_string(),
            },
            None => Self::DirectCall {
                peer: channel_id.to_string(),
            },
        }
    }
}

/// Which video track a frame belongs to: one sender sends at most one of
/// each per scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Track {
    Camera,
    Screen,
}

impl Track {
    /// Every track, in stream-derivation order.
    pub const ALL: [Self; 2] = [Self::Camera, Self::Screen];

    /// The label a track's stream id is derived from
    /// (`rekindle_video::derive_stream_id`).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Camera => "camera",
            Self::Screen => "screen",
        }
    }
}

#[cfg(test)]
#[path = "media_tests.rs"]
mod tests;

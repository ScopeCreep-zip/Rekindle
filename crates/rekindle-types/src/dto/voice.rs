//! Results for actions on the Host's voice session.

use serde::{Deserialize, Serialize};

/// A community voice session the Host is holding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceJoined {
    pub joined: bool,
    pub community: String,
    pub channel: String,
    pub muted: bool,
    pub deafened: bool,
}

/// A community voice session the Host has ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceLeft {
    pub left: bool,
    pub community: String,
    pub channel: String,
}

/// The controls of the live session after applying the requested change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceControls {
    pub muted: bool,
    pub deafened: bool,
}

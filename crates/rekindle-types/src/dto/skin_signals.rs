//! A skin window's state signals (plan SK2-A4 step 3, design §2.5): the
//! Host computes them from its own state and the window applies the
//! state triggers they name (`OnActivate`, `OnAppCanChat`,
//! `OnActiveUserTyping`, `OnAppVoiceRinging`, `OnDisconnected`,
//! `OnNewNotification`, …). The frontend never computes a capability.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::dto::skin_action::Capability;

/// Every [`Capability`] (Xfire's `OnAppCan<X>` / `OnAppCannot<X>`), in
/// order: each is in exactly one of [`SkinSignalSet::can`] and
/// [`SkinSignalSet::cannot`].
pub const EVERY_CAPABILITY: [Capability; 26] = [
    Capability::AddFriend,
    Capability::Chat,
    Capability::Profile,
    Capability::Join,
    Capability::VoiceChat,
    Capability::HostVoiceChat,
    Capability::LeaveVoiceChat,
    Capability::CancelVoiceChat,
    Capability::MuteSelf,
    Capability::UnmuteSelf,
    Capability::AddServer,
    Capability::RefreshServer,
    Capability::StartDownload,
    Capability::StopDownload,
    Capability::ExecuteDownload,
    Capability::HostGameLobby,
    Capability::JoinGameLobby,
    Capability::LaunchGameLobby,
    Capability::LeaveGameLobby,
    Capability::CreateRoom,
    Capability::EnterRoom,
    Capability::RenameRoom,
    Capability::Delete,
    Capability::DeleteSelected,
    Capability::OpenSelected,
    Capability::UploadSelected,
];

/// A Host-fired signal a pack trigger can pair with `SetState(Disabled)`
/// (`OnDisconnected`, `OnActiveUserTyping`, `OnAppVoiceRinging`,
/// `OnAppUserStartedTalking`, `OnNewNotification`): the signals
/// [`SkinSignalSet`] carries besides the capabilities and the focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DisableSignal {
    Disconnected,
    ActiveUserTyping,
    VoiceRinging,
    UserStartedTalking,
    NewNotification,
}

/// Why a node can be disabled now: a capability in
/// [`SkinSignalSet::cannot`], or an active [`DisableSignal`]. On the wire
/// the bare name (`"chat"`, `"disconnected"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DisableCause {
    Capability(Capability),
    Signal(DisableSignal),
}

/// One skin window's signals.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkinSignalSet {
    /// The window has the OS focus (`OnActivate`; `OnDeactivate` when not).
    pub focused: bool,
    /// What the selection allows (`OnAppCan<X>`).
    pub can: BTreeSet<Capability>,
    /// What it does not (`OnAppCannot<X>`); all of them with no selection.
    pub cannot: BTreeSet<Capability>,
    /// The selected friend is typing to us (`OnActiveUserTyping`).
    pub user_typing: bool,
    /// A call with the selected friend is ringing (`OnAppVoiceRinging`).
    pub voice_ringing: bool,
    /// The selected friend is talking in a call (`OnAppUserStartedTalking`).
    pub talking: bool,
    /// The node is not attached to the network (`OnDisconnected`).
    pub disconnected: bool,
    /// A notification arrived since the window last had focus
    /// (`OnNewNotification`).
    pub new_notification: bool,
    /// The Host's words for each cause that can disable a node now (plan
    /// E7.7 CR1): every capability in `cannot` and every active
    /// [`DisableSignal`], each with the reason the window shows.
    pub disabled_reasons: BTreeMap<DisableCause, String>,
}

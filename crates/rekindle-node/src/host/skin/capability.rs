//! Each skin capability from real state (plan SK2-A4 step 3; SK2-G design
//! part (5) of each feature and §11 G1's selection variants; owner
//! 2026-11-21, lead decision 49). A capability is `cannot` only for a real
//! state condition, as Xfire's `OnAppCannot<X>` were (nothing selected,
//! the friend offline, a game we cannot launch, no download running),
//! never because a feature is missing.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use rekindle_game_detect::GameDatabase;
use rekindle_types::dto::skin_action::Capability;
use rekindle_types::dto::skin_run::SkinSelection;
use rekindle_types::dto::transfer::TransferRow;
use rekindle_types::friend::{FriendState, FriendshipState, UserStatus};

use super::signals::Inputs;

/// What a window has selected, its friend resolved in the session.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Selected<'a> {
    Nothing,
    Friend {
        key: &'a str,
        friend: Option<&'a FriendState>,
    },
    Server {
        game_id: u32,
        address: &'a str,
    },
    Transfer {
        attachment_id: &'a str,
    },
    Media {
        path: &'a str,
    },
    Group {
        group_id: &'a str,
    },
    /// A community window (plan E7.7 HSC1): its community, channel and
    /// selected game server.
    Community {
        community: &'a str,
        channel: Option<&'a str>,
        server: Option<&'a str>,
    },
}

impl<'a> Selected<'a> {
    /// `selection` with its friend looked up in `friend`.
    pub(crate) fn of(
        selection: Option<&'a SkinSelection>,
        friend: Option<&'a FriendState>,
    ) -> Self {
        match selection {
            // A friend group's header is no capability's object: nothing
            // a pack's buttons act on is selected (its page is the
            // Infoview's, plan E7.15 IV5).
            None | Some(SkinSelection::FriendGroup { .. }) => Self::Nothing,
            Some(SkinSelection::Friend { public_key }) => Self::Friend {
                key: public_key,
                friend,
            },
            Some(SkinSelection::Server { game_id, address }) => Self::Server {
                game_id: *game_id,
                address,
            },
            Some(SkinSelection::Transfer { attachment_id, .. }) => Self::Transfer { attachment_id },
            Some(SkinSelection::Media { path }) => Self::Media { path },
            Some(SkinSelection::Group { group_id }) => Self::Group { group_id },
            Some(SkinSelection::Community {
                community,
                channel,
                server,
            }) => Self::Community {
                community,
                channel: channel.as_deref(),
                server: server.as_deref(),
            },
        }
    }

    /// The selected key, when a friend (or a key) is selected.
    pub(crate) fn key(self) -> Option<&'a str> {
        match self {
            Self::Friend { key, .. } => Some(key),
            _ => None,
        }
    }
}

/// A group DM as the capabilities see it (`group_dms`, `group_dm_invites`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Group {
    /// We are in it, with this many other members; `creator` when we
    /// created it (the roster's first member, `rekindle_dm::GroupRoster::
    /// creator`), the one member whose rename the others accept.
    /// `lobby`: its lobby's verdicts for us (`group_lobby::current`).
    Joined {
        others: usize,
        creator: bool,
        lobby: Option<LobbyCaps>,
    },
    /// We are invited and have not joined.
    Pending,
}

/// A group's lobby as the capabilities see it: whether it is open, and
/// the Host's join, leave and launch verdicts for us
/// (`group_dm::lobby::verdicts`, the ones `LobbyView` carries).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LobbyCaps {
    pub open: bool,
    pub join: bool,
    pub leave: bool,
    pub launch: bool,
}

/// The state besides the selection and the Host's stream, read before
/// each recompute (`signals::load_world`).
#[derive(Debug, Clone, Default)]
pub(crate) struct SkinWorld {
    pub accepted_friends: usize,
    /// Accepted friends whose presence names a server.
    pub friend_servers: usize,
    /// Our `favourite_servers`, as (game id, `host:port`).
    pub favourites: BTreeSet<(u32, String)>,
    pub groups: BTreeMap<String, Group>,
    /// The friends whose chat a group was created from (SK2-G §6).
    pub attached: BTreeSet<String>,
    pub communities: usize,
    /// Our shown game's id.
    pub own_game: Option<u32>,
    /// Our `transfers` (plan E7.2 G12), by attachment id.
    pub transfers: BTreeMap<String, TransferRow>,
    /// The own-records stage is `Held`: our profile and mailbox are
    /// published, so a friend code, invite or request can name them
    /// (`FriendStores::own_records`, plan E2.5 FC3).
    pub records_held: bool,
    /// The joined communities, by id, with their voice channels and game
    /// servers (a community window's capabilities, `super::community`).
    pub community_views: BTreeMap<String, super::community::SkinCommunity>,
}

/// A community window's verdict on `capability` (`community::verdict`):
/// `None` when the selection is no community or the community scope
/// leaves the capability to the general table.
fn community_verdict(
    capability: Capability,
    inputs: &Inputs,
    world: &SkinWorld,
    selected: Selected<'_>,
) -> Option<super::community::CommunityVerdict> {
    let Selected::Community {
        community,
        channel,
        server,
    } = selected
    else {
        return None;
    };
    let view = world.community_views.get(community);
    super::community::verdict(capability, inputs, community, view, channel, server)
}

/// The game can be launched to a server (`launcher::joins_servers`, the
/// launcher's own condition): a connect template, or a Steam app id with
/// Xfire's network arguments (plan E7.2 G2).
pub(crate) fn launchable(game_id: u32) -> bool {
    GameDatabase::shared()
        .lookup_by_id(game_id)
        .is_some_and(rekindle_game_detect::launcher::joins_servers)
}

fn media_exists(selected: Selected<'_>) -> bool {
    matches!(selected, Selected::Media { path } if Path::new(path).is_file())
}

/// An online accepted friend whose game the Host's launch check judges
/// joinable here (`journal_entry::not_joinable`, plan E7.15 IV3c: a parsed
/// server and a launch method), the verdict `JoinFriend` refuses on
/// (SK2-G §1, lead decision 49).
fn can_join_friend(friend: Option<&FriendState>) -> bool {
    friend.is_some_and(|f| {
        f.friendship_state == FriendshipState::Accepted
            && f.status != UserStatus::Offline
            && f.game_info.as_ref().is_some_and(|g| {
                crate::host::journal_entry::not_joinable(
                    Some(g.game_id),
                    g.server_address.as_deref(),
                )
                .is_none()
            })
    })
}

/// The selected friend's capabilities (chat, profile, calls).
fn friend_allows(capability: Capability, inputs: &Inputs, selected: Selected<'_>) -> bool {
    let Selected::Friend { key, friend } = selected else {
        return false;
    };
    let accepted = friend.is_some_and(|f| f.friendship_state == FriendshipState::Accepted);
    let online = accepted && friend.is_some_and(|f| f.status != UserStatus::Offline);
    let call = inputs.call_with(key);
    match capability {
        Capability::Chat => accepted,
        // Xfire's `HostVoiceChat` places a call (`ChatButtonCall`) and its
        // `VoiceChat` answers one (`ChatButtonAnswerCall`, art
        // `button_call_ringing`, tip `ButtonAnswerCallTip`; Symbiosis
        // `ChatWindow.xml:280-295`, `Common.xml:450-456`).
        Capability::HostVoiceChat => online && call.is_none(),
        Capability::VoiceChat => call.is_some_and(|c| c.ringing && !c.outgoing),
        Capability::LeaveVoiceChat => call.is_some_and(|c| !c.ringing),
        Capability::CancelVoiceChat => call.is_some_and(|c| c.ringing && c.outgoing),
        // Profile and Delete act on any selected key.
        _ => true,
    }
}

/// A group window's call capabilities (Xfire pages `TabbedGCCall` by
/// them: Host, Join, Leave; Symbiosis `Components/GroupChatWindow.xml:
/// 112-118`, WMP10/Alloy/LOTR/DiabloIILoD `GroupChatInfoskin.ini`):
/// `HostVoiceChat` starts the group's call while it has none and another
/// member, `VoiceChat` joins its call in progress that we are not in,
/// `LeaveVoiceChat` holds while we are in it, `CancelVoiceChat` while our
/// own call of it still rings out.
fn group_call_allows(
    capability: Capability,
    inputs: &Inputs,
    world: &SkinWorld,
    group_id: &str,
) -> bool {
    let joined =
        matches!(world.groups.get(group_id), Some(Group::Joined { others, .. }) if *others >= 1);
    if !joined {
        return false;
    }
    let call = inputs.group_call(group_id).map(|(_, c)| c);
    match capability {
        Capability::HostVoiceChat => call.is_none(),
        Capability::VoiceChat => call.is_some_and(|c| c.ringing && !c.outgoing),
        Capability::LeaveVoiceChat => call.is_some_and(|c| !c.ringing),
        Capability::CancelVoiceChat => call.is_some_and(|c| c.ringing && c.outgoing),
        _ => false,
    }
}

fn lobby_of(group: Option<Group>) -> Option<LobbyCaps> {
    match group {
        Some(Group::Joined { lobby, .. }) => lobby,
        _ => None,
    }
}

/// Whether `capability` holds for a window with `selected`.
pub(crate) fn allows(
    capability: Capability,
    inputs: &Inputs,
    world: &SkinWorld,
    selected: Selected<'_>,
) -> bool {
    use Capability as C;
    if let Some(verdict) = community_verdict(capability, inputs, world, selected) {
        return verdict.words().is_none();
    }
    let group_id = match selected {
        Selected::Group { group_id } => Some(group_id),
        _ => None,
    };
    let group = group_id.and_then(|g| world.groups.get(g).copied());
    match capability {
        // Xfire's Add Friend works whatever is selected (plan SKC1): it
        // holds while our profile and mailbox are published, the stage
        // every friend code, invite and request waits on and is refused
        // without (`operations::friend::invites::published_identity`).
        C::AddFriend => world.records_held,
        // A group window's call tab (`TabbedGCCall`) follows its group.
        C::VoiceChat | C::HostVoiceChat | C::LeaveVoiceChat | C::CancelVoiceChat
            if group_id.is_some() =>
        {
            group_id.is_some_and(|g| group_call_allows(capability, inputs, world, g))
        }
        C::Chat
        | C::Profile
        | C::Delete
        | C::VoiceChat
        | C::HostVoiceChat
        | C::LeaveVoiceChat
        | C::CancelVoiceChat => friend_allows(capability, inputs, selected),
        C::MuteSelf => inputs.in_call() && !inputs.self_muted(),
        C::UnmuteSelf => inputs.in_call() && inputs.self_muted(),
        // A friend's server, or a server in the view, that we can launch.
        C::Join => match selected {
            Selected::Friend { friend, .. } => can_join_friend(friend),
            Selected::Server { game_id, .. } => launchable(game_id),
            _ => false,
        },
        // In a 1:1 window Xfire's `OnAppCanJoin` drives it (SK2-G §5).
        // In a group window (SK2-G §5 (5)): an open lobby hosted by another
        // member that we are not in.
        C::JoinGameLobby => match selected {
            Selected::Friend { friend, .. } => can_join_friend(friend),
            _ => lobby_of(group).is_some_and(|l| l.join),
        },
        // We are in it and are not its host.
        C::LeaveGameLobby => lobby_of(group).is_some_and(|l| l.leave),
        // We host it and it holds at least one joined member.
        C::LaunchGameLobby => lobby_of(group).is_some_and(|l| l.launch),
        // A group with another member and no open lobby, while we are in a
        // game we can launch.
        C::HostGameLobby => {
            matches!(group, Some(Group::Joined { others, .. }) if others >= 1)
                && !lobby_of(group).is_some_and(|l| l.open)
                && world.own_game.is_some_and(launchable)
        }
        C::AddServer => match selected {
            Selected::Server { game_id, address } => {
                !world.favourites.contains(&(game_id, address.to_owned()))
            }
            _ => false,
        },
        // A server selected, or any server we know of.
        C::RefreshServer => {
            matches!(selected, Selected::Server { .. })
                || world.friend_servers > 0
                || !world.favourites.is_empty()
        }
        // A paused or failed transfer, or an offered attachment not yet
        // fetched (SK2-G §4 (5)).
        C::StartDownload => match selected {
            Selected::Transfer { attachment_id } => world
                .transfers
                .get(attachment_id)
                .is_none_or(|t| t.state.resumable()),
            _ => false,
        },
        C::StopDownload => match selected {
            Selected::Transfer { attachment_id } => world
                .transfers
                .get(attachment_id)
                .is_some_and(|t| t.state.running()),
            _ => false,
        },
        // Done, and its file still at the stored path.
        C::ExecuteDownload => match selected {
            Selected::Transfer { attachment_id } => world
                .transfers
                .get(attachment_id)
                .is_some_and(|t| crate::host::transfers::revealable(t).is_some()),
            _ => false,
        },
        // A group needs us and two accepted friends (`MIN_MEMBERS` 3).
        C::CreateRoom => world.accepted_friends >= 2,
        // A selected group, or the room attached to the selected chat.
        C::EnterRoom => {
            group.is_some()
                || matches!(selected, Selected::Friend { key, .. } if world.attached.contains(key))
        }
        // Only the creator names the room (lead decision 1a; the others
        // drop a rename from anyone else, `group_dm/rename.rs`).
        C::RenameRoom => matches!(group, Some(Group::Joined { creator: true, .. })),
        C::OpenSelected => media_exists(selected),
        // Deleting a room is leaving it (SK2-G §6).
        C::DeleteSelected => media_exists(selected) || matches!(group, Some(Group::Joined { .. })),
        // Something to share it with: a friend, a group or a community.
        C::UploadSelected => {
            media_exists(selected)
                && (world.accepted_friends > 0
                    || world
                        .groups
                        .values()
                        .any(|g| matches!(g, Group::Joined { .. }))
                    || world.communities > 0)
        }
    }
}

/// Why `capability` is refused for a window with `selected`, `None` while
/// it holds: the one source of a capability's refusal words (plan E7.7
/// CR1, owner ruling: a disabled button shows the Host's reason). The
/// signal set's `disabled_reasons` carries exactly these words for every
/// capability in `cannot`.
pub(crate) fn refusal(
    capability: Capability,
    inputs: &Inputs,
    world: &SkinWorld,
    selected: Selected<'_>,
) -> Option<String> {
    if let Some(verdict) = community_verdict(capability, inputs, world, selected) {
        return verdict.words().map(str::to_owned);
    }
    (!allows(capability, inputs, world, selected))
        .then(|| why(capability, inputs, selected).to_owned())
}

/// The selected friend's refusals (the conditions of [`friend_allows`]).
fn friend_why(capability: Capability, selected: Selected<'_>) -> &'static str {
    let Selected::Friend { friend, .. } = selected else {
        return "Select a friend first.";
    };
    let accepted = friend.is_some_and(|f| f.friendship_state == FriendshipState::Accepted);
    let online = accepted && friend.is_some_and(|f| f.status != UserStatus::Offline);
    match capability {
        Capability::Chat => "You can chat once they accept your friend request.",
        Capability::VoiceChat | Capability::HostVoiceChat if !accepted => {
            "You can call once they accept your friend request."
        }
        Capability::VoiceChat | Capability::HostVoiceChat if !online => "They're offline.",
        Capability::VoiceChat | Capability::HostVoiceChat => "You're already in a call with them.",
        Capability::LeaveVoiceChat => "There's no connected call with them to leave.",
        Capability::CancelVoiceChat => "There's no outgoing call ringing to cancel.",
        _ => "Select a friend first.",
    }
}

/// The words for a refused `capability` (the conditions of [`allows`]).
fn why(capability: Capability, inputs: &Inputs, selected: Selected<'_>) -> &'static str {
    use Capability as C;
    let friend = matches!(selected, Selected::Friend { .. });
    let server = matches!(selected, Selected::Server { .. });
    let transfer = matches!(selected, Selected::Transfer { .. });
    match capability {
        C::AddFriend => "Your profile is still being published. Adding friends opens once it is.",
        C::Chat
        | C::Profile
        | C::Delete
        | C::VoiceChat
        | C::HostVoiceChat
        | C::LeaveVoiceChat
        | C::CancelVoiceChat => friend_why(capability, selected),
        C::MuteSelf | C::UnmuteSelf if !inputs.in_call() => "You're not in a call.",
        C::MuteSelf => "You're already muted.",
        C::UnmuteSelf => "You're not muted.",
        C::Join | C::JoinGameLobby if friend => {
            "Your friend isn't in a game you can join from here."
        }
        C::Join if server => "Rekindle can't launch this game to a server.",
        C::Join => "Select a friend or a server first.",
        C::JoinGameLobby => "There's no open lobby here to join.",
        C::LeaveGameLobby => "You're not in a lobby you can leave.",
        C::LaunchGameLobby => "You can launch only a lobby you host, once someone has joined.",
        C::HostGameLobby => {
            "Hosting a lobby needs a room with another member, no open lobby, and a game you're playing that Rekindle can launch."
        }
        C::AddServer if server => "This server is already in your favourites.",
        C::AddServer => "Select a server first.",
        C::RefreshServer => "There are no servers to refresh.",
        C::StartDownload if transfer => "This download is already running or done.",
        C::StopDownload if transfer => "This download isn't running.",
        C::ExecuteDownload if transfer => "This download isn't finished, or its file has moved.",
        C::StartDownload | C::StopDownload | C::ExecuteDownload => "Select a download first.",
        C::CreateRoom => "A room needs at least two friends.",
        C::EnterRoom => "Select a room, or a chat that has one.",
        C::RenameRoom => "Only the room's creator can rename it.",
        C::UploadSelected if media_exists(selected) => {
            "There's no one to share it with yet: add a friend, or join a room or a community."
        }
        C::OpenSelected | C::UploadSelected => "Select a picture or video first.",
        C::DeleteSelected => "Select a picture, a video or a room first.",
    }
}

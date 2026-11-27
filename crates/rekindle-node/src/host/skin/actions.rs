//! The one table from a pack's actions to real operations (plan SK2-A4,
//! design §2.2): every feature action is the `IpcRequest` today's handler
//! sends, run through the intent gate (ADR 0019) by `HostApi::call`
//! ([`resolve`]); window, menu, tile and panel operations are
//! [`ShellOp`]s the window's shell runs; an action Rekindle has no feature
//! for yet answers [`SkinRun::NotBuiltYet`] (each listed with its plan
//! step, lead decision 49). A selection-dependent action acts
//! on the window's selection the Host keeps ([`Selections`]), never on the
//! click target, and is refused while nothing is selected.

use std::collections::BTreeMap;

use rekindle_ipc::protocol::{
    GroupDmRequest, IpcRequest, IpcResponse, LobbyRequest, ServersRequest, TransferRequest,
};
use rekindle_types::dto::local_search::LocalSearchScope;
use rekindle_types::dto::skin_action::{EngineView, MenuRegistry, SkinAction as A};
use rekindle_types::dto::skin_run::{CallOp, ShellOp, ShellPanel, SkinRun, SkinSelection};
use rekindle_types::dto::transfer::TransferTarget;
use rekindle_types::friend::GameInfoState;
use rekindle_types::game_server::{JoinSource, ServerAddress};

pub(crate) use super::launchers::Launchers;
use super::reads::Reads;
use super::rooms::{self, Room};

/// Each window's selection (`SkinSelectionSet`), by window label (lead
/// decision 48): a chat window's is its own peer, the buddy list's its
/// selected friend; one shared per kind would act on, and show the
/// capabilities of, another window's friend.
#[derive(Debug, Default)]
pub(crate) struct Selections(parking_lot::Mutex<BTreeMap<String, SkinSelection>>);

impl Selections {
    pub(crate) fn set(&self, window: &str, selection: Option<SkinSelection>) {
        let mut all = self.0.lock();
        match selection {
            Some(s) => all.insert(window.to_owned(), s),
            None => all.remove(window),
        };
    }

    /// The window's selected friend, when its selection is a friend.
    pub(crate) fn friend(&self, window: &str) -> Option<String> {
        match self.0.lock().get(window) {
            Some(SkinSelection::Friend { public_key }) => Some(public_key.clone()),
            _ => None,
        }
    }

    /// The window's selected transfer, when its selection is one.
    pub(crate) fn transfer(&self, window: &str) -> Option<(String, TransferTarget)> {
        match self.0.lock().get(window) {
            Some(SkinSelection::Transfer {
                attachment_id,
                target,
            }) => Some((attachment_id.clone(), target.clone())),
            _ => None,
        }
    }

    /// The window's selected game server, as (game id, `host:port`).
    pub(crate) fn server(&self, window: &str) -> Option<(u32, String)> {
        match self.0.lock().get(window) {
            Some(SkinSelection::Server { game_id, address }) => Some((*game_id, address.clone())),
            _ => None,
        }
    }

    /// The window's selected community, when its selection is one.
    pub(crate) fn community(&self, window: &str) -> Option<String> {
        match self.0.lock().get(window) {
            Some(SkinSelection::Community { community, .. }) => Some(community.clone()),
            _ => None,
        }
    }

    /// The window's selection.
    pub(crate) fn get(&self, window: &str) -> Option<SkinSelection> {
        self.0.lock().get(window).cloned()
    }

    /// The labels of the windows that have a selection.
    pub(crate) fn windows(&self) -> Vec<String> {
        self.0.lock().keys().cloned().collect()
    }
}

/// Each window's current view, by window label (SK2-G §8): the last
/// `ShowView` the Host admitted for it, recorded on the window that shows
/// it (a view switch pressed in a popup is its launcher's, [`addressed`]).
/// `SearchFor` searches what that view lists.
#[derive(Debug, Default)]
pub(crate) struct Views(parking_lot::Mutex<BTreeMap<String, EngineView>>);

impl Views {
    /// Record the view `run` shows: in `elsewhere`'s window when it was
    /// handed on, else in `window`, the window pressed.
    pub(crate) fn record(
        &self,
        window: &str,
        run: &SkinRun,
        elsewhere: Option<&(String, ShellOp)>,
    ) {
        let (shown_in, op) = match (elsewhere, run) {
            (Some((to, op)), _) => (to.as_str(), op),
            (None, SkinRun::Shell(op)) => (window, op),
            (None, _) => return,
        };
        if let ShellOp::ShowView(view) = op {
            self.0.lock().insert(shown_in.to_owned(), *view);
        }
    }

    /// What `SearchFor` pressed in `window` searches. A chat window
    /// (`chat-g<group id>`, or `chat-…` whose selection is its own peer,
    /// lead decision 48) searches its conversation; any other window the
    /// view it (or, for a popup, its `launcher`) shows, the buddy list's
    /// Friends until a view was shown. `None` for a view with nothing local
    /// to search yet (the pictures and videos, plan E7.2 G14).
    pub(crate) fn scope(
        &self,
        window: &str,
        launcher: Option<&str>,
        selections: &Selections,
    ) -> Option<LocalSearchScope> {
        if let Some(chat) = window.strip_prefix("chat-") {
            let conversation_id = match chat.strip_prefix('g') {
                Some(group) => group.to_owned(),
                None => selections.friend(window)?,
            };
            return Some(LocalSearchScope::Conversation { conversation_id });
        }
        let shown = self.0.lock().get(launcher.unwrap_or(window)).copied();
        match shown.unwrap_or(EngineView::Friends) {
            EngineView::Servers => Some(LocalSearchScope::Servers),
            EngineView::Files => Some(LocalSearchScope::Files),
            EngineView::Videos | EngineView::Screenshots => None,
            EngineView::Home
            | EngineView::Friends
            | EngineView::Chats
            | EngineView::ChatRooms
            | EngineView::Notifications
            | EngineView::Options
            | EngineView::Help
            | EngineView::InviteUsersToRoom
            | EngineView::Communities => Some(LocalSearchScope::Friends),
        }
    }
}

/// The notifications inbox belongs to the buddy list. A view switch in a
/// popup belongs to its launcher (plan SK2-N4). For operations addressed
/// to another window, the caller receives [`SkinRun::Launcher`] and the
/// destination receives the [`ShellOp`] on its channel.
pub(crate) fn addressed(
    launchers: &Launchers,
    window: &str,
    run: SkinRun,
) -> (SkinRun, Option<(String, ShellOp)>) {
    const BUDDY_LIST: &str = "buddy-list";
    let SkinRun::Shell(op) = run else {
        return (run, None);
    };
    let to = match op {
        ShellOp::OpenPanel(ShellPanel::Notifications) if window != BUDDY_LIST => {
            Some(BUDDY_LIST.to_owned())
        }
        ShellOp::ShowView(_) | ShellOp::GroupPage { .. } => launchers.of(window),
        _ => None,
    };
    match to {
        Some(to) => (SkinRun::Launcher { window: to.clone() }, Some((to, op))),
        None => (SkinRun::Shell(op), None),
    }
}

/// Where an action goes.
#[derive(Debug, Clone)]
pub(crate) enum Mapped {
    Request(Box<IpcRequest>),
    Run(SkinRun),
}

/// The window's selections one action may act on.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Selected<'a> {
    pub friend: Option<&'a str>,
    pub transfer: Option<&'a (String, TransferTarget)>,
    pub server: Option<&'a (u32, String)>,
    /// What `SearchFor` searches ([`Views::scope`]).
    pub search: Option<&'a LocalSearchScope>,
    pub community: Option<&'a str>,
}

/// `JoinFriend` (SK2-G §1, lead decision 49): the selected friend's game
/// on the server their presence shares, launched by the one launch path
/// (`GameLaunch`, TrustChanging for a peer's address). Refused with the
/// Host's own `not_joinable` reason, the check the `Join` capability makes.
fn join_friend(friend: &str, game: Option<&GameInfoState>) -> Result<Mapped, IpcResponse> {
    let refused = |why: String| IpcResponse::error(409, format!("JoinFriend: {friend} {why}"));
    let game = game.ok_or_else(|| refused("is in no game".into()))?;
    let server = game.server_address.as_deref();
    if let Some(reason) = crate::host::journal_entry::not_joinable(Some(game.game_id), server) {
        return Err(refused(format!("cannot be joined: {reason:?}")));
    }
    let server = server
        .map(ServerAddress::parse)
        .and_then(Result::ok)
        .ok_or_else(|| refused("shares no server".into()))?;
    Ok(Mapped::Request(Box::new(IpcRequest::GameLaunch {
        game_id: game.game_id,
        server: Some(server),
        source: JoinSource::FriendPresence,
    })))
}

/// `AddServer` (SK2-G §3): the selected server into our favourites, the
/// existing `FavouriteServerAdd` (labelled later from the Servers view).
fn add_server(server: Option<&(u32, String)>) -> Result<Mapped, IpcResponse> {
    let (game_id, address) = server.ok_or_else(|| {
        IpcResponse::error(
            409,
            "AddServer acts on the selected server: none is selected",
        )
    })?;
    let server = ServerAddress::parse(address)
        .map_err(|e| IpcResponse::error(400, format!("AddServer: {address}: {e}")))?;
    Ok(Mapped::Request(Box::new(IpcRequest::Servers(
        ServersRequest::FavouriteAdd {
            game_id: *game_id,
            server,
            label: String::new(),
        },
    ))))
}

/// The table. `selected` holds the window's selected friend, transfer
/// and server, `reads` what the room and join actions read
/// ([`super::reads::load`]).
///
/// # Errors
/// 409 for a selection-dependent action while nothing it acts on is
/// selected.
pub(crate) fn map(
    action: &A,
    selected: Selected<'_>,
    reads: &Reads,
) -> Result<Mapped, IpcResponse> {
    let Selected {
        friend,
        transfer,
        server,
        search,
        community,
    } = selected;
    // A community window's pack row (plan E7.7 HSC1, `super::community`).
    if let Some(mapped) = reads
        .community
        .as_ref()
        .and_then(|c| super::community::map(action, c))
    {
        return mapped;
    }
    let rooms = &reads.rooms;
    use Mapped::{Request, Run};
    let shell = |op| Ok(Run(SkinRun::Shell(op)));
    let panel = |p| shell(ShellOp::OpenPanel(p));
    let selected = || {
        friend.map(str::to_owned).ok_or_else(|| {
            IpcResponse::error(
                409,
                format!("{action:?} acts on the selected friend: none is selected"),
            )
        })
    };
    let transfer_selected = || {
        transfer.cloned().ok_or_else(|| {
            IpcResponse::error(
                409,
                format!("{action:?} acts on the selected transfer: none is selected"),
            )
        })
    };
    let transfer_request = |r| Ok(Request(Box::new(IpcRequest::Transfer(r))));
    match action {
        // Feature requests (the handlers' own requests, intent-gated).
        // The Downloads view (plan E7.2 G12, SK2-G §4): Xfire's Execute is
        // a reveal; "Search for Files" lists our transfers.
        A::StartDownload => {
            let (attachment_id, target) = transfer_selected()?;
            transfer_request(TransferRequest::Start {
                attachment_id,
                target,
                dest: None,
            })
        }
        A::StopDownload => transfer_request(TransferRequest::Stop {
            attachment_id: transfer_selected()?.0,
        }),
        A::OpenDownload => transfer_request(TransferRequest::Open {
            attachment_id: transfer_selected()?.0,
        }),
        A::DownloadInfo => transfer_request(TransferRequest::List),
        // SK2-G §8: the local search, scoped by the window's view.
        A::SearchFor => match search {
            Some(scope) => shell(ShellOp::OpenSearch {
                scope: scope.clone(),
            }),
            None => Ok(gap(action)),
        },
        A::LogOff => Ok(Request(Box::new(IpcRequest::Logout))),
        A::RefreshServer => Ok(Request(Box::new(IpcRequest::Servers(
            ServersRequest::Refresh,
        )))),
        A::SetStatus(status) => Ok(Request(Box::new(IpcRequest::PresenceSet {
            status: *status,
        }))),
        // Leave acts on the window's call (`reads.in_call`): a group
        // window's own (whose Cancel and End end our ring), a chat
        // window's 1:1; anywhere else it is the community voice channel's.
        A::LeaveVoiceChat | A::LeaveGroupVoiceChat | A::CancelVoiceChat | A::EndVoiceChat
            if reads.in_call.is_some() =>
        {
            rooms::leave(reads.in_call.as_ref())
        }
        A::LeaveVoiceChat | A::LeaveGroupVoiceChat => Ok(Request(Box::new(IpcRequest::VoiceLeave))),
        A::LaunchUrl(u) => Ok(Request(Box::new(IpcRequest::LinkOpen {
            url: u.url.clone(),
        }))),
        // Selection-dependent shell operations.
        A::ChatFriend | A::SendMessage => shell(ShellOp::OpenChat {
            friend: selected()?,
        }),
        A::ProfileFriend | A::ViewProfile => shell(ShellOp::OpenProfile {
            friend: selected()?,
        }),
        A::ShowSafetyNumber => shell(ShellOp::OpenSafetyNumber {
            friend: selected()?,
        }),
        A::ShowGameInvite => shell(ShellOp::OpenGameInvite {
            friend: selected()?,
        }),
        // Xfire's `AppHostVoiceChat` places a call (`ChatButtonCall`) and
        // `AppVoiceChat` answers the ringing one (`ChatButtonAnswerCall`,
        // Symbiosis `ChatWindow.xml:291-293`, `Common.xml:450-452`).
        // A group window draws the same action on its Host button
        // (Symbiosis `GroupChatWindow`'s `ChatButtonCall`, action
        // `hostVoiceChat`): there it starts the group's call, as
        // `AppHostGroupVoiceChat` does, never a 1:1 call.
        A::HostVoiceChat if reads.group_call.is_some() => {
            Ok(rooms::host_group(reads.group_call.as_ref()))
        }
        A::HostVoiceChat => shell(ShellOp::Call(CallOp::Voice {
            friend: selected()?,
        })),
        // A group window's join (`GroupChatButtonAnswerCall`, tip
        // `ButtonGroupJoinVoiceChatTip`) runs `AppVoiceChat` in Symbiosis
        // (`Components/GroupChatWindow.xml:144-147`) and
        // `AppGroupVoiceChat` in WMP10/Alloy/LOTR/DiabloIILoD; its Host
        // button (`GroupChatButtonCall`) runs `AppHostGroupVoiceChat`.
        A::GroupVoiceChat => rooms::join_group(reads.group_call.as_ref()),
        A::VoiceChat if reads.group_call.is_some() => rooms::join_group(reads.group_call.as_ref()),
        A::VoiceChat => {
            selected()?;
            reads.ringing_call.clone().map_or_else(
                || Err(IpcResponse::error(409, "No call from them is ringing.")),
                |call_id| shell(ShellOp::Call(CallOp::Answer { call_id })),
            )
        }
        A::VideoChat => shell(ShellOp::Call(CallOp::Video {
            friend: selected()?,
        })),
        A::CancelVoiceChat | A::EndVoiceChat => shell(ShellOp::Call(CallOp::End)),
        A::MuteSelf => shell(ShellOp::Call(CallOp::Mute)),
        A::UnmuteSelf => shell(ShellOp::Call(CallOp::Unmute)),
        // Panels (today's dialogs).
        A::AddFriend => panel(ShellPanel::AddFriend),
        A::NewChat => panel(ShellPanel::NewChat),
        A::NewGroup => panel(ShellPanel::NewGroup),
        // Rooms are group DMs (SK2-G §6).
        A::NewRoom | A::CreateRoom => shell(ShellOp::NewRoom {
            peer: friend.map(str::to_owned),
        }),
        A::EnterRoom | A::JoinRoom | A::EnterAttachedRoom => rooms::enter(action, rooms),
        A::InviteUsersToRoom => rooms::invite(rooms),
        // Game lobbies (SK2-G §5): with no room selected Host opens the
        // new-group panel; a 1:1 window's Join is Xfire's `OnAppCanJoin`.
        A::HostGameLobby => match &rooms.selected {
            Some(Room::Joined(group_id)) => {
                Ok(Request(Box::new(IpcRequest::Lobby(LobbyRequest::Host {
                    group_id: group_id.clone(),
                }))))
            }
            _ => panel(ShellPanel::NewGroup),
        },
        A::JoinGameLobby if rooms.selected.is_none() => {
            join_friend(&selected()?, reads.friend_game.as_ref())
        }
        A::JoinGameLobby | A::LeaveGameLobby | A::LaunchGameLobby => rooms::lobby(action, rooms),
        // Deleting a room is leaving it (SK2-G §6); a picture or video's
        // delete is the media library's (E7.2 G14).
        A::DeleteSelected => match &rooms.selected {
            Some(Room::Joined(group_id)) => Ok(Request(Box::new(IpcRequest::GroupDm(
                GroupDmRequest::Leave {
                    group_id: group_id.clone(),
                },
            )))),
            _ => Ok(gap(action)),
        },
        A::JoinFriend => join_friend(&selected()?, reads.friend_game.as_ref()),
        A::AddServer => add_server(server),
        // A group window's host (`GroupChatButtonCall`) opens the panel
        // scoped to its group, prefilled from the roster; the buddy
        // list's has no group and opens the plain panel.
        A::HostGroupVoiceChat => Ok(rooms::host_group(reads.group_call.as_ref())),
        A::OpenCommunity | A::LeaveCommunity => super::community::row(action, community),
        A::StartCommunity => panel(ShellPanel::CommunityStart),
        A::CreateCommunity => panel(ShellPanel::CreateCommunity),
        A::JoinCommunity => panel(ShellPanel::JoinCommunity),
        A::ShowShortcuts => panel(ShellPanel::Shortcuts),
        A::OpenNotifications | A::ShowView(EngineView::Notifications) => {
            panel(ShellPanel::Notifications)
        }
        A::ShowSettings(tab) => shell(ShellOp::ShowSettings(*tab)),
        A::ReportBug => shell(ShellOp::ReportBug),
        A::CopyKey => shell(ShellOp::CopyKey),
        A::EditName => shell(ShellOp::EditName),
        A::Quit => shell(ShellOp::Quit),
        // Window, menu and tile operations.
        A::DragWindow => shell(ShellOp::DragWindow),
        A::Minimize => shell(ShellOp::Minimize),
        A::Maximize => shell(ShellOp::Maximize),
        A::Close => shell(ShellOp::Close),
        A::Resize(e) => shell(ShellOp::Resize(*e)),
        A::Menu(m) | A::ContextMenu(m) => {
            if MenuRegistry::FEATURE_GAPS.contains(m) {
                Ok(gap(action))
            } else {
                shell(ShellOp::OpenMenu {
                    menu: *m,
                    context: matches!(action, A::ContextMenu(_)),
                })
            }
        }
        A::ShowView(v) => shell(ShellOp::ShowView(*v)),
        A::GroupPage { group, page } => shell(ShellOp::GroupPage {
            group: *group,
            page: *page,
        }),
        A::ShowTile(t) => shell(ShellOp::ShowTile(t.clone())),
        A::HideTile(t) => shell(ShellOp::HideTile(t.clone())),
        A::LaunchPopup(p) => shell(ShellOp::LaunchPopup(p.clone())),
        A::Animate(s) => shell(ShellOp::Animate(*s)),
        A::AnimateStop => shell(ShellOp::AnimateStop),
        A::AnimationStart { group, spec } => shell(ShellOp::AnimationStart {
            group: *group,
            spec: *spec,
        }),
        A::AnimationStop { group } => shell(ShellOp::AnimationStop { group: *group }),
        A::SetState(s) => shell(ShellOp::SetState(*s)),
        A::PlaySound(s) => shell(ShellOp::PlaySound(s.clone())),
        // The active call's window, else our community voice session's
        // (plan E7.2 G-item A); refused while there is neither.
        A::ShowVoiceWindow => reads.show_call.clone().map_or_else(
            || {
                Err(IpcResponse::error(
                    409,
                    "There's no call or voice session to show.",
                ))
            },
            |target| shell(ShellOp::ShowCall { target }),
        ),
        // Not built yet, each with its plan step (lead decision 49; the
        // table in SK2-T1a's mapping): the button stays; the window says so.
        A::OpenSelected
        | A::UploadSelected
        | A::ShowWindow
        | A::HideWindow
        | A::DialogOk
        | A::DialogCancel
        | A::EnableOverlay(_)
        | A::DisableOverlay(_)
        | A::UpdateClock
        | A::TackWindow
        | A::UntackWindow
        | A::OpenOverlayOptions
        | A::AttachedClose
        | A::BrowseBack
        | A::BrowseForward => Ok(gap(action)),
    }
}

/// A feature gap's answer, with the words the window shows: the action's
/// user-facing name and, where known, the plan row that builds it.
fn gap(action: &A) -> Mapped {
    let (name, row) = gap_name(action);
    let message = match row {
        Some(row) => format!("{name} isn't in Rekindle yet (plan {row})"),
        None => format!("{name} isn't in Rekindle yet"),
    };
    Mapped::Run(SkinRun::NotBuiltYet {
        action: action.clone(),
        message,
    })
}

/// Each gap action's user-facing name and plan row (the table in SK2-T1a's
/// mapping: lobbies G6, media G14, search G11, overlay G15–G18, menus G9).
fn gap_name(action: &A) -> (String, Option<&'static str>) {
    const OVERLAY: Option<&str> = Some("E7.2 G15–G18");
    let name = |s: &str| s.to_owned();
    match action {
        A::OpenSelected => (name("Opening a picture or video"), Some("E7.2 G14")),
        A::UploadSelected => (name("Uploading a picture or video"), Some("E7.2 G14")),
        A::DeleteSelected => (name("Deleting a picture or video"), Some("E7.2 G14")),
        // Search answers a gap only in the pictures and videos views (G11).
        A::SearchFor => (name("Searching pictures and videos"), Some("E7.2 G14")),
        A::ShowWindow => (name("Showing the in-game overlay"), OVERLAY),
        A::HideWindow => (name("Hiding the in-game overlay"), OVERLAY),
        A::DialogOk | A::DialogCancel => (name("The overlay's dialogs"), OVERLAY),
        A::EnableOverlay(_) | A::DisableOverlay(_) => {
            (name("Turning overlay parts on and off"), OVERLAY)
        }
        A::UpdateClock => (name("The overlay clock"), OVERLAY),
        A::TackWindow | A::UntackWindow => (name("Pinning overlay windows"), OVERLAY),
        A::OpenOverlayOptions => (name("Overlay options"), OVERLAY),
        A::AttachedClose => (name("Closing an attached overlay window"), OVERLAY),
        A::BrowseBack | A::BrowseForward => (name("The overlay browser"), OVERLAY),
        A::Menu(m) | A::ContextMenu(m) => (format!("The {m:?} menu"), Some("E7.2 G9")),
        other => (format!("{other:?}"), None),
    }
}

/// What [`resolve`] made of a request.
#[derive(Debug)]
pub(crate) enum Resolved {
    /// Not a skin action, or a request action's own request.
    Request(IpcRequest),
    /// A `RunSkinAction` that is no request, with its answer: the request
    /// still meets its clearance and the gate, and `HostApi::call` answers
    /// with `run` once they admit it.
    Run { request: IpcRequest, run: SkinRun },
}

/// `RunSkinAction` in `HostApi::call`, before clearance: the one place an
/// action is resolved (with the [`super::reads::load`] it reads). A request action
/// becomes its request (which then meets its own clearance and the intent
/// gate); any other stays `RunSkinAction` with its run.
pub(crate) fn resolve(
    skin: &super::SkinState,
    reads: &Reads,
    request: IpcRequest,
) -> Result<Resolved, IpcResponse> {
    let IpcRequest::RunSkinAction { window, action, .. } = &request else {
        return Ok(Resolved::Request(request));
    };
    let selections = &skin.selection;
    let at = skin.launchers.selection_window(window);
    let friend = selections.friend(&at);
    let transfer = selections.transfer(&at);
    let server = selections.server(&at);
    let community = selections.community(&at);
    let launcher = skin.launchers.of(window);
    let search = skin.views.scope(window, launcher.as_deref(), selections);
    let selected = Selected {
        friend: friend.as_deref(),
        transfer: transfer.as_ref(),
        server: server.as_ref(),
        search: search.as_ref(),
        community: community.as_deref(),
    };
    match map(action, selected, reads)? {
        Mapped::Request(mapped) => Ok(Resolved::Request(*mapped)),
        Mapped::Run(run) => Ok(Resolved::Run { request, run }),
    }
}

/// The router's answer to an admitted `RunSkinAction` (its run was
/// resolved by [`resolve`]; `HostApi::call` answers with it), and to
/// `SkinSelectionSet`.
pub(crate) fn answer(selections: &Selections, request: IpcRequest) -> IpcResponse {
    match request {
        IpcRequest::RunSkinAction { .. } => IpcResponse::ok(&()),
        IpcRequest::SkinSelectionSet { window, selection } => {
            selections.set(&window, selection);
            IpcResponse::ok(&())
        }
        other => unreachable!("{} is not a skin action request", other.name()),
    }
}

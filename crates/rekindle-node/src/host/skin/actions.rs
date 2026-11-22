//! The one table from a pack's actions to real operations (plan SK2-A4,
//! design §2.2): every feature action is the `IpcRequest` today's handler
//! sends, run through the intent gate (ADR 0019) by `HostApi::call`
//! ([`resolve`]); window, menu, tile and panel operations are
//! [`ShellOp`]s the window's shell runs; an action Rekindle has no feature
//! for answers [`SkinRun::NotBuiltYet`]. A selection-dependent action acts
//! on the window's selection the Host keeps ([`Selections`]), never on the
//! click target, and is refused while nothing is selected.

use std::collections::BTreeMap;

use rekindle_ipc::protocol::{IpcRequest, IpcResponse, TransferRequest};
use rekindle_types::dto::skin_action::{MenuRegistry, SkinAction as A};
use rekindle_types::dto::skin_run::{CallOp, ShellOp, ShellPanel, SkinRun, SkinSelection};
use rekindle_types::dto::transfer::TransferTarget;

use super::rooms::{self, Rooms};

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

    /// The window's selection.
    pub(crate) fn get(&self, window: &str) -> Option<SkinSelection> {
        self.0.lock().get(window).cloned()
    }

    /// The labels of the windows that have a selection.
    pub(crate) fn windows(&self) -> Vec<String> {
        self.0.lock().keys().cloned().collect()
    }
}

/// Where an action goes.
#[derive(Debug, Clone)]
pub(crate) enum Mapped {
    Request(Box<IpcRequest>),
    Run(SkinRun),
}

/// The table. `friend` and `transfer` are the window's selected friend
/// and transfer, `rooms` what the room actions read ([`rooms::load`]).
///
/// # Errors
/// 409 for a selection-dependent action while nothing it acts on is
/// selected.
pub(crate) fn map(
    action: &A,
    friend: Option<&str>,
    transfer: Option<&(String, TransferTarget)>,
    rooms: &Rooms,
) -> Result<Mapped, IpcResponse> {
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
        A::LogOff => Ok(Request(Box::new(IpcRequest::Logout))),
        A::SetStatus(status) => Ok(Request(Box::new(IpcRequest::PresenceSet {
            status: *status,
        }))),
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
        A::VoiceChat | A::HostVoiceChat => shell(ShellOp::Call(CallOp::Voice {
            friend: selected()?,
        })),
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
        A::GroupVoiceChat | A::HostGroupVoiceChat => panel(ShellPanel::StartGroupCall),
        A::CreateCommunity => panel(ShellPanel::CreateCommunity),
        A::JoinCommunity => panel(ShellPanel::JoinCommunity),
        A::ShowShortcuts => panel(ShellPanel::Shortcuts),
        A::OpenNotifications => panel(ShellPanel::Notifications),
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
        // Feature gaps (design §2.2): each button stays; the window says so.
        A::JoinFriend
        | A::ShowVoiceWindow
        | A::HostGameLobby
        | A::JoinGameLobby
        | A::LeaveGameLobby
        | A::LaunchGameLobby
        | A::AddServer
        | A::RefreshServer
        | A::InviteUsersToRoom
        | A::DeleteSelected
        | A::OpenSelected
        | A::UploadSelected
        | A::SearchFor
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

fn gap(action: &A) -> Mapped {
    Mapped::Run(SkinRun::NotBuiltYet {
        action: action.clone(),
    })
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
/// action is resolved (with the [`rooms::load`] it reads). A request action
/// becomes its request (which then meets its own clearance and the intent
/// gate); any other stays `RunSkinAction` with its run.
pub(crate) fn resolve(
    selections: &Selections,
    rooms: &Rooms,
    request: IpcRequest,
) -> Result<Resolved, IpcResponse> {
    let IpcRequest::RunSkinAction { window, action, .. } = &request else {
        return Ok(Resolved::Request(request));
    };
    match map(
        action,
        selections.friend(window).as_deref(),
        selections.transfer(window).as_ref(),
        rooms,
    )? {
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

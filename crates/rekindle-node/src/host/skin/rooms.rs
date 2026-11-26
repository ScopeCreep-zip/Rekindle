//! Rooms are group DMs (SK2-G §6, plan E7.2 G7). There is no room
//! directory and no public room (threat model): a room is a group we were
//! invited to or created. `EnterRoom`/`JoinRoom` act on the window's
//! selected group (a joined one opens, a pending invite is accepted, the
//! TrustChanging `GroupDmAccept`), and on a friend's chat they, like
//! `EnterAttachedRoom`, open the group most recently created from it
//! (`group_dms.origin_peer`).

use rekindle_ipc::protocol::{GroupDmRequest, IpcRequest, IpcResponse, LobbyRequest};
use rekindle_transport::operations::group_dm;
use rekindle_types::dto::group_lobby::LobbyView;
use rekindle_types::dto::skin_action::SkinAction as A;
use rekindle_types::dto::skin_run::{CallOp, ShellOp, ShellPanel, SkinRun, SkinSelection};

use super::actions::{Mapped, Selections};
use super::reads::{GroupCall, InCall};
use crate::daemon::dispatch::DaemonContext;

/// The selected group's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Room {
    Joined(String),
    Pending(String),
}

/// What the room actions read: the window's selected group, and the room
/// attached to its selected friend's chat.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Rooms {
    pub selected: Option<Room>,
    pub attached: Option<String>,
    /// The selected joined room's lobby (`group_lobby::current`), as we see it.
    pub lobby: Option<LobbyView>,
}

/// The actions that read the rooms: the room actions, and
/// `DeleteSelected`, which on a joined group leaves it (SK2-G §6).
fn is_room(action: &A) -> bool {
    matches!(
        action,
        A::EnterRoom
            | A::JoinRoom
            | A::EnterAttachedRoom
            | A::DeleteSelected
            | A::InviteUsersToRoom
            | A::HostGameLobby
            | A::JoinGameLobby
            | A::LeaveGameLobby
            | A::LaunchGameLobby
    )
}

/// The room actions' answer.
///
/// # Errors
/// 409 when no group is selected and no room is attached to the chat.
pub(crate) fn enter(action: &A, rooms: &Rooms) -> Result<Mapped, IpcResponse> {
    let open = |group_id: &String| {
        Ok(Mapped::Run(SkinRun::Shell(ShellOp::OpenGroupChat {
            group_id: group_id.clone(),
        })))
    };
    match (action, &rooms.selected, &rooms.attached) {
        (A::EnterAttachedRoom, _, Some(group_id))
        | (A::EnterRoom | A::JoinRoom, Some(Room::Joined(group_id)), _)
        | (A::EnterRoom | A::JoinRoom, None, Some(group_id)) => open(group_id),
        (A::EnterRoom | A::JoinRoom, Some(Room::Pending(group_id)), _) => Ok(Mapped::Request(
            Box::new(IpcRequest::GroupDm(GroupDmRequest::Accept {
                group_id: group_id.clone(),
            })),
        )),
        _ => Err(IpcResponse::error(
            409,
            format!("{action:?} acts on a selected room or the chat's room: there is none"),
        )),
    }
}

/// `InviteUsersToRoom` (plan E7.2 G8) on the window's selected joined
/// group: its friend picker, whose choice is one `GroupDmInvite`.
///
/// # Errors
/// 409 for an invite not yet accepted, or no group selected.
/// A lobby action on the selected room's lobby (SK2-G §5): Join, Leave
/// (the host's page is Xfire's "Stop Hosting", rt#780: `LobbyClose`) or
/// Launch on our shown game's server.
pub(crate) fn lobby(action: &A, rooms: &Rooms) -> Result<Mapped, IpcResponse> {
    let (Some(Room::Joined(group_id)), Some(lobby)) = (&rooms.selected, &rooms.lobby) else {
        return Err(IpcResponse::error(
            409,
            format!("{action:?} acts on a selected room's lobby: there is none"),
        ));
    };
    let (group_id, lobby_id) = (group_id.clone(), lobby.lobby_id.clone());
    let request = match action {
        A::JoinGameLobby => IpcRequest::Lobby(LobbyRequest::Join { group_id, lobby_id }),
        A::LeaveGameLobby if lobby.close.allowed() && !lobby.leave.allowed() => {
            IpcRequest::Lobby(LobbyRequest::Close { group_id, lobby_id })
        }
        A::LeaveGameLobby => IpcRequest::Lobby(LobbyRequest::Leave { group_id, lobby_id }),
        _ => IpcRequest::Lobby(LobbyRequest::Launch {
            group_id,
            lobby_id,
            server: None,
        }),
    };
    Ok(Mapped::Request(Box::new(request)))
}

pub(crate) fn invite(rooms: &Rooms) -> Result<Mapped, IpcResponse> {
    match &rooms.selected {
        Some(Room::Joined(group_id)) => Ok(Mapped::Run(SkinRun::Shell(ShellOp::InviteToRoom {
            group_id: group_id.clone(),
        }))),
        Some(Room::Pending(_)) => Err(IpcResponse::error(
            409,
            "InviteUsersToRoom: accept the invite first",
        )),
        None => Err(IpcResponse::error(
            409,
            "InviteUsersToRoom acts on a selected room: there is none",
        )),
    }
}

/// [`Rooms`] for a room action from the database; empty for any other
/// request (nothing read).
///
/// # Errors
/// 500 when the groups cannot be read.
pub(crate) async fn load(
    ctx: &DaemonContext,
    selections: &Selections,
    request: &IpcRequest,
) -> Result<Rooms, IpcResponse> {
    let IpcRequest::RunSkinAction { window, action, .. } = request else {
        return Ok(Rooms::default());
    };
    let Some(selection) = selections.get(window).filter(|_| is_room(action)) else {
        return Ok(Rooms::default());
    };
    let (Ok(db), Ok(owner)) = (ctx.require_db(), ctx.require_owner()) else {
        return Ok(Rooms::default());
    };
    let now = ctx.clock.now_ms_i64();
    let me = owner.clone();
    let (mut rooms, row) = db
        .call(move |conn| {
            use rekindle_db::repo::{group_dm_invites, group_dms, group_lobby};
            Ok(match selection {
                SkinSelection::Group { group_id } => {
                    let selected = if group_dms::get(conn, &owner, &group_id)?.is_some() {
                        Some(Room::Joined(group_id))
                    } else if group_dm_invites::list(conn, &owner, now)?
                        .iter()
                        .any(|i| i.group_id == group_id)
                    {
                        Some(Room::Pending(group_id))
                    } else {
                        None
                    };
                    let row = match &selected {
                        Some(Room::Joined(id)) => group_lobby::current(conn, &owner, id)?,
                        _ => None,
                    };
                    let rooms = Rooms {
                        selected,
                        attached: None,
                        lobby: None,
                    };
                    (rooms, row)
                }
                SkinSelection::Friend { public_key } => (
                    Rooms {
                        selected: None,
                        attached: group_dms::attached(conn, &owner, &public_key)?,
                        lobby: None,
                    },
                    None,
                ),
                _ => (Rooms::default(), None),
            })
        })
        .await
        .map_err(|e| IpcResponse::error(500, format!("the rooms cannot be read: {e}")))?;
    rooms.lobby = row
        .map(|r| group_dm::lobby::view_of(r, &me))
        .transpose()
        .map_err(|e| IpcResponse::error(500, format!("the room's lobby cannot be read: {e}")))?;
    Ok(rooms)
}

/// A group window's join: the selected group's call in progress that we
/// are not in, joined through the shell (`CallOp::JoinGroup`).
///
/// # Errors
/// 409 when no group is selected, or the group has no call to join.
pub(crate) fn join_group(group: Option<&GroupCall>) -> Result<Mapped, IpcResponse> {
    match group {
        Some(GroupCall {
            group_id,
            call_id: Some(call_id),
            ..
        }) => Ok(Mapped::Run(SkinRun::Shell(ShellOp::Call(
            CallOp::JoinGroup {
                group_id: group_id.clone(),
                call_id: call_id.clone(),
            },
        )))),
        Some(_) => Err(IpcResponse::error(
            409,
            "No call in this group is in progress.",
        )),
        None => Err(IpcResponse::error(
            409,
            "GroupVoiceChat acts on the selected group: none is selected",
        )),
    }
}

/// A window's leave (and a group window's cancel): the group's call
/// through `CallOp::LeaveGroup`, a chat window's 1:1 call through
/// `CallOp::Leave`.
///
/// # Errors
/// 409 when the window is in no call, the reason its capability is off.
pub(crate) fn leave(in_call: Option<&InCall>) -> Result<Mapped, IpcResponse> {
    let op = match in_call {
        Some(InCall::Group {
            group_id,
            call_id: Some(call_id),
        }) => CallOp::LeaveGroup {
            group_id: group_id.clone(),
            call_id: call_id.clone(),
        },
        Some(InCall::Direct {
            call_id: Some(call_id),
        }) => CallOp::Leave {
            call_id: call_id.clone(),
        },
        Some(InCall::Group { call_id: None, .. }) => {
            return Err(IpcResponse::error(409, "You're not in this group's call."))
        }
        Some(InCall::Direct { call_id: None }) | None => {
            return Err(IpcResponse::error(409, "You're not in a call with them."))
        }
    };
    Ok(Mapped::Run(SkinRun::Shell(ShellOp::Call(op))))
}

/// A group window's host (`AppHostGroupVoiceChat`, and the `hostVoiceChat`
/// Symbiosis draws on its Host button): the Start Group Call panel scoped
/// to the selected group and prefilled from its roster; with no group
/// selected (the buddy list) the plain panel.
pub(crate) fn host_group(group: Option<&GroupCall>) -> Mapped {
    Mapped::Run(SkinRun::Shell(match group {
        Some(g) => ShellOp::StartGroupCall {
            group_id: g.group_id.clone(),
            group_name: g.group_name.clone(),
            members: g.members.clone(),
        },
        None => ShellOp::OpenPanel(ShellPanel::StartGroupCall),
    }))
}

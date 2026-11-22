//! Rooms are group DMs (SK2-G §6, plan E7.2 G7). There is no room
//! directory and no public room (threat model): a room is a group we were
//! invited to or created. `EnterRoom`/`JoinRoom` act on the window's
//! selected group (a joined one opens, a pending invite is accepted, the
//! TrustChanging `GroupDmAccept`), and on a friend's chat they, like
//! `EnterAttachedRoom`, open the group most recently created from it
//! (`group_dms.origin_peer`).

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::dto::skin_action::SkinAction as A;
use rekindle_types::dto::skin_run::{ShellOp, SkinRun, SkinSelection};

use super::actions::{Mapped, Selections};
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
}

/// The actions that read the rooms: the room actions, and
/// `DeleteSelected`, which on a joined group leaves it (SK2-G §6).
fn is_room(action: &A) -> bool {
    matches!(
        action,
        A::EnterRoom | A::JoinRoom | A::EnterAttachedRoom | A::DeleteSelected
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
        (A::EnterRoom | A::JoinRoom, Some(Room::Pending(group_id)), _) => {
            Ok(Mapped::Request(Box::new(IpcRequest::GroupDmAccept {
                group_id: group_id.clone(),
            })))
        }
        _ => Err(IpcResponse::error(
            409,
            format!("{action:?} acts on a selected room or the chat's room: there is none"),
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
    db.call(move |conn| {
        use rekindle_db::repo::{group_dm_invites, group_dms};
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
                Rooms {
                    selected,
                    attached: None,
                }
            }
            SkinSelection::Friend { public_key } => Rooms {
                selected: None,
                attached: group_dms::attached(conn, &owner, &public_key)?,
            },
            _ => Rooms::default(),
        })
    })
    .await
    .map_err(|e| IpcResponse::error(500, format!("the rooms cannot be read: {e}")))
}

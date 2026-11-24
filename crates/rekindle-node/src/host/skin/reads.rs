//! What the action table reads besides the window's selection (plan
//! SK2-A4; lead decision 49): the rooms ([`rooms::load`]) and the selected
//! friend's game as the Host's presence holds it, with the Host's own
//! `joinable` verdict (`journal_entry::not_joinable`, plan E7.15 IV3c), so
//! `JoinFriend` launches exactly what the capability offered.

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::dto::skin_action::SkinAction as A;
use rekindle_types::dto::skin_run::{CallTarget, GroupCallInvitee, SkinSelection};
use rekindle_types::friend::GameInfoState;

use super::actions::Selections;
use super::rooms::{self, Rooms};
use crate::daemon::dispatch::DaemonContext;

/// The reads one action resolves against.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Reads {
    pub rooms: Rooms,
    /// The selected friend's game, from the session's friend state.
    pub friend_game: Option<GameInfoState>,
    /// `ShowVoiceWindow`'s window: the Host's active call, else our
    /// community voice session, folded from its stream (`signals`).
    pub show_call: Option<CallTarget>,
    /// `VoiceChat`'s call: the incoming call from the selected friend that
    /// is ringing (`signals::Inputs::ringing_from`).
    pub ringing_call: Option<String>,
    /// A group window's `GroupVoiceChat` (and its `VoiceChat`) and
    /// `HostGroupVoiceChat`: the selected group, its roster, and its call
    /// in progress that we are not in (`signals::Inputs::joinable_group_call`).
    pub group_call: Option<GroupCall>,
}

/// The selected group and the call of its that we can join, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GroupCall {
    pub group_id: String,
    pub group_name: String,
    /// The group's members besides us, in roster order: the Start Group
    /// Call panel's prefill.
    pub members: Vec<GroupCallInvitee>,
    pub call_id: Option<String>,
}

/// [`GroupCall`] for a group window's join or host: the group from
/// `group_dms`, and its call by the group id its signaling carries.
async fn group_call(
    ctx: &DaemonContext,
    selections: &Selections,
    request: &IpcRequest,
) -> Result<Option<GroupCall>, IpcResponse> {
    let IpcRequest::RunSkinAction {
        window,
        action: A::VoiceChat | A::GroupVoiceChat | A::HostGroupVoiceChat,
        ..
    } = request
    else {
        return Ok(None);
    };
    let Some(SkinSelection::Group { group_id }) = selections.get(window) else {
        return Ok(None);
    };
    let (Ok(db), Ok(owner)) = (ctx.require_db(), ctx.require_owner()) else {
        return Ok(None);
    };
    let id = group_id.clone();
    let group = db
        .call(move |conn| rekindle_db::repo::group_dms::get(conn, &owner, &id))
        .await
        .map_err(|e| IpcResponse::error(500, format!("the group cannot be read: {e}")))?;
    let Some(group) = group else {
        return Ok(None);
    };
    let owner = ctx.require_owner().ok();
    let members = group
        .roster
        .into_iter()
        .filter(|m| Some(&m.identity) != owner.as_ref())
        .map(|m| GroupCallInvitee {
            pubkey: m.identity,
            display_name: m.display_name,
        })
        .collect();
    let call_id = ctx.skin.signals.joinable_group_call(&group.group_id);
    Ok(Some(GroupCall {
        group_id: group.group_id,
        group_name: group.name,
        members,
        call_id,
    }))
}

/// [`Reads`] for `request`; empty for anything but a `RunSkinAction`.
///
/// # Errors
/// 500 when the rooms or the selected group cannot be read.
pub(crate) async fn load(
    ctx: &DaemonContext,
    selections: &Selections,
    request: &IpcRequest,
) -> Result<Reads, IpcResponse> {
    let rooms = rooms::load(ctx, selections, request).await?;
    let friend_game = match request {
        IpcRequest::RunSkinAction {
            window,
            action: A::JoinFriend,
            ..
        } => selections.friend(window).and_then(|key| {
            ctx.session
                .read()
                .as_ref()
                .and_then(|s| s.friends.get(&key))
                .and_then(|f| f.game_info.clone())
        }),
        _ => None,
    };
    let show_call = match request {
        IpcRequest::RunSkinAction {
            action: A::ShowVoiceWindow,
            ..
        } => ctx.skin.signals.show_call(),
        _ => None,
    };
    let ringing_call = match request {
        IpcRequest::RunSkinAction {
            window,
            action: A::VoiceChat,
            ..
        } => selections
            .friend(window)
            .and_then(|key| ctx.skin.signals.ringing_from(&key)),
        _ => None,
    };
    let group_call = group_call(ctx, selections, request).await?;
    Ok(Reads {
        rooms,
        friend_game,
        show_call,
        ringing_call,
        group_call,
    })
}

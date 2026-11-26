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
    /// A group window's `GroupVoiceChat` (and its `VoiceChat`),
    /// `HostGroupVoiceChat` and the `HostVoiceChat` Symbiosis draws on its
    /// Host button: the selected group, its roster, and its call in
    /// progress that we are not in (`signals::Inputs::joinable_group_call`).
    pub group_call: Option<GroupCall>,
    /// The call a window's Leave (and a group window's Cancel and End)
    /// acts on ([`in_call`]); `None` elsewhere, where Leave is the
    /// community voice channel's.
    pub in_call: Option<InCall>,
    /// A community window's selection, its community read from the joined
    /// ones (`super::community::read`).
    pub community: Option<super::community::CommunityRead>,
}

/// The call one window's leave acts on, by the window's kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InCall {
    /// A group window (selection `Group`): its group, and our call of it,
    /// one we are in or our own still ringing out
    /// (`signals::Inputs::our_group_call`).
    Group {
        group_id: String,
        call_id: Option<String>,
    },
    /// A 1:1 chat window (`chat-…`, its peer selected): our connected call
    /// with that peer (`signals::Inputs::call_in_with`).
    Direct { call_id: Option<String> },
}

/// [`InCall`] for `action` pressed in `window` with `selection`.
/// `our_group_call` and `call_in_with` are the call registry the
/// capabilities read (`signals::Inputs`), so Leave acts on exactly the
/// call that enabled it. A 1:1 window's Cancel and End stay `CallOp::End`.
pub(crate) fn in_call(
    window: &str,
    action: &A,
    selection: Option<SkinSelection>,
    our_group_call: impl Fn(&str) -> Option<String>,
    call_in_with: impl Fn(&str) -> Option<String>,
) -> Option<InCall> {
    let leave = matches!(action, A::LeaveVoiceChat | A::LeaveGroupVoiceChat);
    if !leave && !matches!(action, A::CancelVoiceChat | A::EndVoiceChat) {
        return None;
    }
    match selection? {
        SkinSelection::Group { group_id } => Some(InCall::Group {
            call_id: our_group_call(&group_id),
            group_id,
        }),
        SkinSelection::Friend { public_key } if leave && window.starts_with("chat-") => {
            Some(InCall::Direct {
                call_id: call_in_with(&public_key),
            })
        }
        _ => None,
    }
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
/// `HostVoiceChat` is read too: Symbiosis's `GroupChatWindow` draws it on
/// its Host button (`ChatButtonCall`), and only a `Group` selection yields
/// a group, so a 1:1 window's `HostVoiceChat` keeps its 1:1 call.
async fn group_call(
    ctx: &DaemonContext,
    selections: &Selections,
    request: &IpcRequest,
) -> Result<Option<GroupCall>, IpcResponse> {
    let IpcRequest::RunSkinAction {
        window,
        action: A::VoiceChat | A::GroupVoiceChat | A::HostGroupVoiceChat | A::HostVoiceChat,
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
    let in_call = match request {
        IpcRequest::RunSkinAction { window, action, .. } => {
            let signals = &ctx.skin.signals;
            in_call(
                window,
                action,
                selections.get(window),
                |g| signals.our_group_call(g),
                |p| signals.call_in_with(p),
            )
        }
        _ => None,
    };
    let community = match request {
        IpcRequest::RunSkinAction { window, .. } => {
            let selection = selections.get(window);
            match &selection {
                Some(SkinSelection::Community { community, .. }) => {
                    let views = super::community::load(ctx, Some(community)).await;
                    super::community::read(selection.as_ref(), |id| views.get(id).cloned())
                }
                _ => None,
            }
        }
        _ => None,
    };
    Ok(Reads {
        rooms,
        friend_game,
        show_call,
        ringing_call,
        group_call,
        in_call,
        community,
    })
}

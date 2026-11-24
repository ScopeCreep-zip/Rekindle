//! What the action table reads besides the window's selection (plan
//! SK2-A4; lead decision 49): the rooms ([`rooms::load`]) and the selected
//! friend's game as the Host's presence holds it, with the Host's own
//! `joinable` verdict (`journal_entry::not_joinable`, plan E7.15 IV3c), so
//! `JoinFriend` launches exactly what the capability offered.

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::dto::skin_action::SkinAction as A;
use rekindle_types::dto::skin_run::CallTarget;
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
}

/// [`Reads`] for `request`; empty for anything but a `RunSkinAction`.
///
/// # Errors
/// 500 when the rooms cannot be read.
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
    Ok(Reads {
        rooms,
        friend_game,
        show_call,
    })
}

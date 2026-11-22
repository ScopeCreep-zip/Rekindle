//! Game invites (plan E7.2), on both hosts: an invite goes to one accepted
//! friend as a ratchet-sealed `GameInvite` (0x0170). Joining one is the
//! Host's `GameLaunch` with `source: Invite` (`JoinGameServer`,
//! TrustChanging), the same act as "Join Game" from a friend's presence.

use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::game_server::ServerAddress;

use super::{state_error, DaemonContext};
use crate::daemon::DaemonState;

/// `GameInviteSend` (`own::route`'s arm): refused for a non-friend, a
/// hidden game, and while `share_current_game` is off. The outbox id.
pub(super) async fn send(
    ctx: &DaemonContext,
    state: DaemonState,
    public_key: &str,
    game_id: u32,
    server: ServerAddress,
) -> IpcResponse {
    if !state.can_write() {
        return state_error(state, "write");
    }
    let friends = crate::host::adapters::friends::HostFriends::of(ctx);
    super::friends::answer(
        rekindle_transport::operations::friend::game_invite::send(
            &friends, public_key, game_id, server,
        )
        .await,
    )
}

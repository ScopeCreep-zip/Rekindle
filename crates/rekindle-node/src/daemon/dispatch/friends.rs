//! Friends, direct messages, session resets and verification on both hosts
//! (plans E2.1, E2.3): each request runs the ported operation
//! (`operations::{friend, dm}`) over the Host (`HostFriends`).

use rekindle_ipc::protocol::{IpcRequest, IpcResponse, Lane};
use rekindle_transport::operations::{dm, friend, friend::FriendError, group_dm};

use super::{state_error, DaemonContext};
use crate::daemon::DaemonState;
use crate::host::adapters::friends::{release_reset_hold, HostFriends};
use crate::host::gate::Acknowledged;

/// The response for a failed friend operation.
fn failed(e: &FriendError) -> IpcResponse {
    let code = match e {
        FriendError::Invalid(_) => 400,
        FriendError::NotFound(_) => 404,
        FriendError::Conflict(_) => 409,
        FriendError::Locked => 423,
        FriendError::NotReady(_) => 503,
        FriendError::Failed(_) => 500,
    };
    IpcResponse::error(code, e.to_string())
}

pub(super) fn answer<T: serde::Serialize>(result: Result<T, FriendError>) -> IpcResponse {
    match result {
        Ok(value) => IpcResponse::ok(&value),
        Err(e) => failed(&e),
    }
}

fn done(result: Result<(), FriendError>) -> IpcResponse {
    answer(result.map(|()| serde_json::json!({ "status": "ok" })))
}

/// Route one friend, DM, session-reset or verification request (the
/// router's arm). An action whose review was held on the peer's verified
/// key changing acknowledges the change with its own write
/// (`acknowledged`).
pub(crate) async fn route(
    ctx: &DaemonContext,
    state: DaemonState,
    request: IpcRequest,
    acknowledged: Acknowledged,
) -> IpcResponse {
    // A query needs a queryable state; a write, an operational one.
    if request.lane() == Lane::Query {
        if !state.can_query() {
            return state_error(state, "query");
        }
    } else if !state.can_write() {
        return state_error(state, "write");
    }
    let host = HostFriends::of(ctx);
    match request {
        IpcRequest::FriendAddFromCode { code, message } => {
            match rekindle_types::friend_code::FriendCode::parse(&code) {
                Ok(code) => done(friend::add(&host, &code, &message).await),
                Err(e) => IpcResponse::error(400, e.to_string()),
            }
        }
        IpcRequest::FriendAccept { public_key } => done(friend::accept(&host, &public_key).await),
        IpcRequest::FriendReject { public_key } => done(friend::reject(&host, &public_key).await),
        IpcRequest::FriendRemove { public_key } => done(friend::remove(&host, &public_key).await),
        IpcRequest::FriendCancelRequest { public_key } => {
            done(friend::cancel(&host, &public_key).await)
        }
        IpcRequest::FriendRetryHandshake { public_key } => done(
            rekindle_transport::operations::prekeys::retry_first_contact(&host, &public_key).await,
        ),
        IpcRequest::FriendList => IpcResponse::ok(&friend::friends(&host)),
        IpcRequest::FriendRequests => answer(friend::pending(&host).await),
        IpcRequest::FriendAddFromInvite { invite } => {
            done(friend::from_invite(&host, &invite).await)
        }
        IpcRequest::FriendInviteCreate => answer(friend::create_invite(&host).await),
        IpcRequest::FriendCode => answer(friend::code::own(&host).await),
        IpcRequest::FriendTokenRotate => answer(friend::code::rotate_token(&host).await),
        IpcRequest::FriendInviteList => answer(friend::invites(&host).await),
        IpcRequest::FriendInviteCancel { invite_id } => {
            done(friend::cancel_invite(&host, &invite_id).await)
        }
        IpcRequest::FriendBlock { public_key } => done(friend::block(&host, &public_key).await),
        IpcRequest::FriendUnblock { public_key } => done(friend::unblock(&host, &public_key).await),
        IpcRequest::FriendBlockedList => answer(friend::blocked(&host).await),
        IpcRequest::FriendSetNickname {
            public_key,
            nickname,
        } => done(friend::set_nickname(&host, &public_key, nickname.as_deref()).await),
        IpcRequest::FriendGroupCreate { name } => answer(
            friend::create_group(&host, &name)
                .await
                .map(|group_id| serde_json::json!({ "groupId": group_id })),
        ),
        IpcRequest::FriendGroupRename { group_id, name } => {
            done(friend::rename_group(&host, group_id, &name).await)
        }
        IpcRequest::FriendGroupMove {
            public_key,
            group_id,
        } => done(friend::move_to_group(&host, &public_key, group_id).await),
        IpcRequest::FriendGroupDelete { group_id } => {
            done(friend::delete_group(&host, group_id).await)
        }
        // A re-initiation the backend held (ruling Q1) is the same reset,
        // run only on the user's commit.
        IpcRequest::SessionResetStart { public_key }
        | IpcRequest::SessionReinitiate { public_key } => {
            done(friend::session_reset::start(&host, &public_key).await)
        }
        IpcRequest::SessionResetAccept { public_key } => {
            let accepted =
                friend::session_reset::accept(&host, &public_key, acknowledged.key_change).await;
            if accepted.is_ok() {
                release_reset_hold(ctx, &public_key);
            }
            done(accepted)
        }
        IpcRequest::SessionResetDecline { public_key } => {
            // The request is answered whether or not the decline reaches them.
            let declined = friend::session_reset::decline(&host, &public_key).await;
            release_reset_hold(ctx, &public_key);
            done(declined)
        }
        IpcRequest::IdentityRotationAccept { public_key } => {
            done(friend::identity_rotation::accept(&host, &public_key).await)
        }
        IpcRequest::IdentityRotationDecline { public_key } => {
            done(friend::identity_rotation::decline(&host, &public_key))
        }
        IpcRequest::IdentitySafetyNumber { peer } => {
            answer(friend::verification::view(&host, &peer).await)
        }
        IpcRequest::IdentityMarkVerified { peer } => {
            done(friend::verification::mark_verified(&host, &peer).await)
        }
        IpcRequest::IdentityClearVerification { peer } => {
            done(friend::verification::clear_verification(&host, &peer).await)
        }
        IpcRequest::DmSend {
            peer_key,
            body,
            reply_to,
        } => answer(
            dm::send_dm(
                &host,
                &peer_key,
                &body,
                reply_to.as_deref(),
                acknowledged.key_change,
            )
            .await,
        ),
        IpcRequest::DmTyping { peer_key, typing } => {
            done(dm::report_draft(&host, &peer_key, typing).await)
        }
        IpcRequest::DmInbox { limit } => answer(dm::inbox(&host, limit).await),
        IpcRequest::DmHistory {
            peer_key,
            limit,
            before,
        } => answer(dm::history(&host, &peer_key, limit, before).await),
        IpcRequest::DmMarkRead {
            peer_key,
            rendered_through,
        } => done(dm::mark_read(&host, &peer_key, rendered_through.as_deref()).await),
        IpcRequest::OutboxRetry { client_id } => {
            done(dm::retry_send(&host, &client_id, acknowledged.key_change).await)
        }
        IpcRequest::OutboxDiscard { client_id } => done(dm::discard_send(&host, &client_id).await),
        IpcRequest::GroupDmCreate { name, members } => {
            answer(group_dm::create(&host, &name, &members).await)
        }
        IpcRequest::GroupDmAccept { group_id } => done(group_dm::accept(&host, &group_id).await),
        IpcRequest::GroupDmDecline { group_id } => done(group_dm::decline(&host, &group_id).await),
        IpcRequest::GroupDmLeave { group_id } => done(group_dm::leave(&host, &group_id).await),
        IpcRequest::GroupDmList => answer(group_dm::list(&host).await),
        IpcRequest::GroupDmMessages {
            group_id,
            limit,
            before,
        } => answer(group_dm::messages(&host, &group_id, limit, before).await),
        other => unreachable!("{} is not a friend or DM request", other.name()),
    }
}

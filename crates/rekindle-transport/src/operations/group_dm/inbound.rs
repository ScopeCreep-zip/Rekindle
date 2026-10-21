//! The receive arms of the group control kinds (plan E2.6): each opened
//! frame goes to its lifecycle step, and the step's reply (our key to a
//! joiner, our rotated key after a leave or decline) goes pairwise.

use rekindle_codec::capnp_codec::direct::DirectBody;

use super::{friend_error, send_pairwise};
use crate::operations::friend::FriendHost;

/// A member's `StreamRecord{group_id}`: the record replaces that member's
/// own group chain; whether it applied. Never the 1:1 path.
pub(crate) async fn on_stream_record(
    host: &dyn FriendHost,
    sender: &str,
    record: &rekindle_codec::capnp_codec::direct::StreamRecord,
) -> bool {
    let (Some(group_id), Ok(deps)) = (record.group_id, host.group_dm()) else {
        return false;
    };
    match rekindle_dm::on_member_chain(deps.as_ref(), sender, &group_id, &record.record_key).await {
        Ok(()) => true,
        Err(e) => {
            tracing::info!(from = %sender, error = %friend_error(&e), "group chain refused");
            false
        }
    }
}

/// Apply an opened group control frame from `sender`; whether it applied.
/// `GroupContent` never travels pairwise (it is a group chain entry), so
/// one that does is refused.
pub async fn apply(host: &dyn FriendHost, sender: &str, body: &DirectBody) -> bool {
    let Ok(deps) = host.group_dm() else {
        return false;
    };
    let deps = deps.as_ref();
    let reply = match body {
        DirectBody::GroupDmInvite(invite) => rekindle_dm::on_invite(deps, sender, invite.clone())
            .await
            .map(|()| None),
        DirectBody::GroupDmAccept(key) => rekindle_dm::on_member_key(deps, sender, key).await,
        DirectBody::GroupDmDecline(d) => rekindle_dm::on_decline(deps, sender, d).await.map(Some),
        DirectBody::GroupDmLeave(l) => rekindle_dm::on_member_left(deps, sender, l).await.map(Some),
        _ => {
            tracing::debug!(from = %sender, kind = ?body.type_id(), "not a pairwise group frame");
            return false;
        }
    };
    if reply.is_ok() && matches!(body, DirectBody::GroupDmLeave(_)) {
        // The leaver is off the roster: their session ends unless they
        // are our friend or share another group with us.
        super::pairwise::drop_sessions(host, &[sender.to_string()]).await;
    }
    match reply {
        Ok(None) => true,
        Ok(Some(out)) => {
            if let Err(e) = send_pairwise(host, out, DirectBody::GroupDmAccept).await {
                tracing::warn!(error = %e, "group key reply not committed");
            }
            true
        }
        Err(e) => {
            tracing::info!(from = %sender, kind = ?body.type_id(), error = %friend_error(&e),
                "group frame refused");
            false
        }
    }
}

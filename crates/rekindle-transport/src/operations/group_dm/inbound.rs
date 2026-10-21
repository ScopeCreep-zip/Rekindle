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
        Ok(()) => {
            super::receive::watch_member(host, &group_id, sender, &record.record_key).await;
            super::receive::spawn_diff(host, &group_id, sender);
            true
        }
        Err(e) => {
            tracing::info!(from = %sender, error = %friend_error(&e), "group chain refused");
            false
        }
    }
}

/// Apply an opened group frame from `sender`; whether it applied. A
/// `GroupContent` pairwise is a member's hint (or typing,
/// [`super::receive::on_hint`]); a member's key, once held, watches and
/// diffs their chain (an entry at that epoch may be waiting for it).
pub async fn apply(host: &dyn FriendHost, sender: &str, body: &DirectBody) -> bool {
    if let DirectBody::GroupContent(content) = body {
        return super::receive::on_hint(host, sender, content).await;
    }
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
        Ok(None) => {
            member_key_held(host, sender, body).await;
            true
        }
        Ok(Some(out)) => {
            member_key_held(host, sender, body).await;
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

/// After a member's key applied: watch the chain it names and diff it.
async fn member_key_held(host: &dyn FriendHost, sender: &str, body: &DirectBody) {
    if let DirectBody::GroupDmAccept(key) = body {
        super::receive::watch_member(host, &key.group_id, sender, &key.chain_record_key).await;
        super::receive::spawn_diff(host, &key.group_id, sender);
    }
}

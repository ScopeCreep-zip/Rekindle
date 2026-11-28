//! The creator-set round's frames on the Host (plan E3.2 S7s.3, decisions
//! (16), (17), (20)), from an accepted friend (the transport's gate):
//! a co-creator invite is held in `creator_invites` and shown; a
//! withdrawal takes it; an accept (its signature verified) or decline from
//! a co-creator our reserved creation waits on is noted for the finish and
//! shown per co-creator; a join link for an invite we accepted joins
//! through the normal join path. A frame naming nothing of ours is refused
//! with a warning, never applied.
//!
//! Lead decision (56): a frame's long-running consequence (the join, or
//! the finish a last answer starts) runs as its own task on the unlock
//! scope (cancelled at lock), never on the per-peer apply path: the DM
//! chain's diff applies one peer's entries in order
//! (`rekindle_sync::stream::StreamEngine::claim`), so an inline join held
//! every later frame from that friend until it ended.

use std::sync::Arc;

use rekindle_transport::operations::friend::creator_round::{self, CreatorFrame};
use rekindle_transport::operations::friend::FriendHost as _;
use rekindle_types::id::PseudonymKey;
use rekindle_types::invite::InviteLink;
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use super::HostFriends;
use crate::host::adapters::governance::HostGovernance;

pub(crate) fn status(host: &HostFriends, peer: &str, anchor: &str, status: &str) {
    host.publish(SubscriptionEvent::Friend(FriendEvent::CoCreatorStatus {
        peer_key: peer.to_owned(),
        anchor: anchor.to_owned(),
        status: status.to_owned(),
    }));
}

/// Apply `frame` from `sender`; whether it was handled (a database failure
/// keeps it for the replay).
pub(super) async fn apply(host: &HostFriends, sender: &str, frame: CreatorFrame) -> bool {
    let (Ok(owner), Ok(db)) = (host.ctx.require_owner(), host.ctx.db.current()) else {
        return false;
    };
    let from = sender.to_owned();
    match frame {
        CreatorFrame::Invite {
            anchor,
            community_name,
        } => {
            let row = rekindle_db::repo::creator_round::CreatorInviteRow {
                anchor: anchor.clone(),
                inviter_key: from,
                community_name: community_name.clone(),
                received_at: host.ctx.clock.now_ms_i64(),
            };
            match db
                .call(move |c| rekindle_db::repo::creator_round::put_invite(c, &owner, &row))
                .await
            {
                Ok(true) => {}
                Ok(false) => return true,
                Err(e) => {
                    tracing::warn!(error = %e, anchor = %anchor, from = %sender,
                        "co-creator invite not stored (kept for the replay)");
                    return false;
                }
            }
            tracing::info!(anchor = %anchor, from = %sender, "co-creator invite held");
            host.publish(SubscriptionEvent::Friend(FriendEvent::CreatorInvited {
                peer_key: sender.to_owned(),
                anchor,
                community_name,
            }));
        }
        CreatorFrame::Withdrawn { anchor } => {
            let a = anchor.clone();
            match db
                .call(move |c| {
                    rekindle_db::repo::creator_round::withdraw_invite(c, &owner, &a, &from)
                })
                .await
            {
                Ok(true) => host.publish(SubscriptionEvent::Friend(
                    FriendEvent::CreatorInviteWithdrawn {
                        peer_key: sender.to_owned(),
                        anchor,
                    },
                )),
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!(error = %e, anchor = %anchor, from = %sender, "co-creator withdrawal not applied");
                    return false;
                }
            }
        }
        CreatorFrame::Accept {
            anchor,
            pseudonym,
            signature,
        } => {
            let key = PseudonymKey(pseudonym);
            if !rekindle_governance::genesis::verify_creator_accept(&anchor, &key, &signature) {
                tracing::warn!(anchor = %anchor, from = %sender, "CreatorAccept with a bad signature refused");
                return true;
            }
            return answer(host, sender, anchor, Some((pseudonym, signature))).await;
        }
        CreatorFrame::Decline { anchor } => return answer(host, sender, anchor, None).await,
        CreatorFrame::Join { anchor, invite } => {
            let a = anchor.clone();
            let taken = db
                .call(move |c| {
                    rekindle_db::repo::creator_round::take_accepted(c, &owner, &a, &from)
                })
                .await;
            match taken {
                // The row is the anchor's one (its key is the anchor) and is
                // taken once: one join task per anchor.
                Ok(Some(_)) => {
                    let ctx = Arc::clone(&host.ctx);
                    spawn(host, "co-creator join", async move {
                        join(ctx, &anchor, &invite).await;
                    });
                }
                Ok(None) => {
                    tracing::warn!(anchor = %anchor, from = %sender, "CreatorJoin for no accepted invite refused");
                }
                Err(e) => {
                    tracing::warn!(error = %e, anchor = %anchor, from = %sender, "co-creator join not applied");
                    return false;
                }
            }
        }
    }
    true
}

/// A co-creator's answer: noted for the finish when our reserved creation
/// of `anchor` waits on it, refused otherwise.
async fn answer(
    host: &HostFriends,
    sender: &str,
    anchor: String,
    accept: Option<([u8; 32], [u8; 64])>,
) -> bool {
    let (Ok(owner), Ok(db)) = (host.ctx.require_owner(), host.ctx.db.current()) else {
        return false;
    };
    let (a, from) = (anchor.clone(), sender.to_owned());
    match db
        .call(move |c| rekindle_db::repo::creator_round::answer_pending(c, &owner, &a, &from))
        .await
    {
        Ok(true) => {
            let accepted = accept.is_some();
            host.ctx
                .friend_stores
                .creator_answers
                .lock()
                .entry(anchor.clone())
                .or_default()
                .insert(sender.to_owned(), accept);
            status(
                host,
                sender,
                &anchor,
                if accepted { "accepted" } else { "declined" },
            );
            // The last answer finishes the creation (its genesis writes):
            // its own task, so the next frame from this friend is not held.
            let governance = HostGovernance::new(Arc::clone(&host.ctx));
            let peer = sender.to_owned();
            spawn(host, "co-creation finish", async move {
                Box::pin(governance.co_creator_settled(&anchor, &peer)).await;
            });
            true
        }
        Ok(false) => {
            tracing::warn!(anchor = %anchor, from = %sender, "co-creator answer for no creation of ours refused");
            true
        }
        Err(e) => {
            tracing::warn!(error = %e, anchor = %anchor, from = %sender, "co-creator answer not applied");
            false
        }
    }
}

/// Run `task` on the unlock scope (lead decision (56)): it ends with the
/// session, and a locked Host starts nothing.
fn spawn<F>(host: &HostFriends, name: &'static str, task: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    if let Err(closed) = host.ctx.unlock_scope_or_closed().spawn(name, task) {
        tracing::warn!(error = %closed, "creator-round task not started: the session ended");
    }
}

/// Join `anchor` with the creator's link (decision (17)): the normal join
/// path, so we count as a member only after our own `MemberJoined`.
async fn join(ctx: Arc<crate::daemon::dispatch::DaemonContext>, anchor: &str, invite: &str) {
    let link = match InviteLink::parse(invite) {
        Ok(link) if link.anchor.as_str() == anchor => link,
        _ => {
            tracing::warn!(anchor = %anchor, "co-creator join link names another community: refused");
            return;
        }
    };
    let governance = HostGovernance::new(ctx);
    match Box::pin(governance.join(&link)).await {
        Ok(_) => governance.activate_created(anchor).await,
        Err(e) => tracing::warn!(community = %anchor, error = ?e, "co-creator join failed"),
    }
}

/// The unlock sweep (decision (20)): a pending-creation row means our
/// creation died before its genesis (its one-time key was never stored),
/// so each co-creator it waited on is sent `CreatorWithdrawn` and the rows
/// are deleted; and every held co-creator invite is shown again, so it can
/// be answered or dismissed.
pub(crate) async fn sweep(host: &HostFriends) {
    let (Ok(owner), Ok(db)) = (host.ctx.require_owner(), host.ctx.db.current()) else {
        return;
    };
    let read = db
        .call(move |c| {
            Ok((
                rekindle_db::repo::creator_round::take_all_pending(c, &owner)?,
                rekindle_db::repo::creator_round::invites(c, &owner)?,
            ))
        })
        .await;
    let (dead, held) = match read {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "creator-round sweep not read");
            return;
        }
    };
    for (anchor, co_creator) in dead {
        let frame = CreatorFrame::Withdrawn {
            anchor: anchor.clone(),
        };
        match creator_round::send(host, &co_creator, &frame).await {
            Ok(_) => status(host, &co_creator, &anchor, "withdrawn"),
            Err(e) => {
                tracing::warn!(error = %e, "CreatorWithdrawn not committed after a dead creation");
            }
        }
    }
    for row in held {
        host.publish(SubscriptionEvent::Friend(FriendEvent::CreatorInvited {
            peer_key: row.inviter_key,
            anchor: row.anchor,
            community_name: row.community_name,
        }));
    }
}

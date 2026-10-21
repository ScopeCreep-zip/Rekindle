//! The selection of a friend, session-reset, verification or 1:1 request
//! (plans E2.1, E2.3): the person it is about, by our nickname and their
//! key fingerprint. An action toward a peer whose verified key changed is
//! held on `VerifiedKeyChanged` until the user acknowledges it.

use rekindle_idempotency::intent_gate::Selection;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_transport::operations::dm;
use rekindle_transport::operations::friend::verification;
use rekindle_types::intent::{Fact, IntentKind, Notifies, Reversibility, Subject};

use super::unavailable;
use crate::daemon::dispatch::DaemonContext;
use crate::host::adapters::friends::HostFriends;

/// Fill `s` for a friend, session-reset or DM request.
///
/// # Errors
/// The invite does not open, a key is not hex, the friend cannot be read,
/// or no session reset from this friend is pending.
pub(super) async fn selection(
    ctx: &DaemonContext,
    request: &IpcRequest,
    s: &mut Selection,
) -> Result<(), IpcResponse> {
    match request {
        IpcRequest::FriendAddFromCode { code, .. } => {
            let code = rekindle_types::friend_code::FriendCode::parse(code)
                .map_err(|e| IpcResponse::error(400, e.to_string()))?;
            s.subject = person(ctx, code.public_key.as_str()).await?;
            s.facts.push(Fact::PresenceVisible);
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::CancelRequest);
            s.notifies = Notifies::Subject;
        }
        IpcRequest::FriendAddFromInvite { invite } => {
            let blob =
                rekindle_transport::operations::friend::open_invite(invite, ctx.clock.now_ms())
                    .map_err(|e| IpcResponse::error(400, e.to_string()))?;
            let fingerprint = hex::decode(&blob.public_key)
                .map(|key| rekindle_crypto::identity::key_fingerprint(&key))
                .map_err(|e| IpcResponse::error(400, format!("invite key: {e}")))?;
            // The inviter's name is theirs to choose: shown, never alone.
            s.subject = Subject::Person {
                local_nickname: None,
                display_name: Some(blob.display_name),
                fingerprint: fingerprint.clone(),
            };
            s.facts.push(Fact::Fingerprint { fingerprint });
            s.facts.push(Fact::PresenceVisible);
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::CancelRequest);
            s.notifies = Notifies::Subject;
        }
        IpcRequest::FriendCancelRequest { public_key } => {
            s.subject = person(ctx, public_key).await?;
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::AddFriend);
        }
        IpcRequest::FriendRetryHandshake { public_key } => {
            // Our accept goes to them again: they are told.
            s.subject = person(ctx, public_key).await?;
            s.notifies = Notifies::Subject;
        }
        IpcRequest::FriendInviteCreate => {
            s.subject = Subject::Named {
                name: "a friend invite link".into(),
            };
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::CancelInvite);
        }
        IpcRequest::FriendTokenRotate => {
            // Every code already shared stops working; friends are untouched.
            s.subject = Subject::Named {
                name: "your friend code".into(),
            };
            s.reversibility = Reversibility::Never;
        }
        IpcRequest::FriendInviteCancel { invite_id } => {
            s.subject = Subject::Named {
                name: invite_id.clone(),
            };
        }
        IpcRequest::FriendBlock { public_key, .. } => {
            s.subject = person(ctx, public_key).await?;
            s.facts.push(Fact::SessionDeleted);
            s.facts.push(Fact::PendingMessagesDropped);
            // Unblocking lets them ask again; the friendship needs them.
            s.reversibility = Reversibility::NeedsOthers;
        }
        IpcRequest::FriendUnblock { public_key } => {
            s.subject = person(ctx, public_key).await?;
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::BlockUser);
        }
        IpcRequest::FriendSetNickname { public_key, .. }
        | IpcRequest::FriendGroupMove { public_key, .. } => {
            s.subject = person(ctx, public_key).await?;
            s.reversibility = if matches!(request, IpcRequest::FriendSetNickname { .. }) {
                Reversibility::Undoable
            } else {
                Reversibility::ReversibleByInverse(IntentKind::MoveFriendToGroup)
            };
        }
        IpcRequest::FriendGroupCreate { name } | IpcRequest::FriendGroupRename { name, .. } => {
            s.subject = Subject::Named { name: name.clone() };
            s.reversibility = if matches!(request, IpcRequest::FriendGroupCreate { .. }) {
                Reversibility::ReversibleByInverse(IntentKind::DeleteFriendGroup)
            } else {
                Reversibility::ReversibleByInverse(IntentKind::RenameFriendGroup)
            };
        }
        IpcRequest::FriendGroupDelete { group_id } => {
            s.subject = Subject::Named {
                name: format!("friend group {group_id}"),
            };
        }
        IpcRequest::SessionResetStart { public_key }
        | IpcRequest::SessionReinitiate { public_key } => {
            s.subject = person(ctx, public_key).await?;
            s.facts.push(Fact::SessionKeptUntilReplaced);
            if matches!(request, IpcRequest::SessionReinitiate { .. }) {
                s.facts.push(Fact::UnopenedMessagesLost);
            }
            s.notifies = Notifies::Subject;
        }
        IpcRequest::SessionResetAccept { public_key }
        | IpcRequest::SessionResetDecline { public_key } => {
            s.subject = person(ctx, public_key).await?;
            let safety_number =
                rekindle_transport::operations::friend::session_reset::pending_safety_number(
                    &HostFriends::of(ctx),
                    public_key,
                )
                .ok_or_else(|| {
                    IpcResponse::error(404, "no session reset from this friend is pending")
                })?;
            s.facts.push(Fact::SafetyNumber { safety_number });
            s.notifies = Notifies::Subject;
            if matches!(request, IpcRequest::SessionResetAccept { .. }) {
                s.escalations.extend(key_change(ctx, public_key).await?);
            }
        }
        IpcRequest::IdentityRotationAccept { public_key }
        | IpcRequest::IdentityRotationDecline { public_key } => {
            s.subject = person(ctx, public_key).await?;
            let (old_safety_number, new_safety_number) =
                rekindle_transport::operations::friend::identity_rotation::pending_safety_numbers(
                    &HostFriends::of(ctx),
                    public_key,
                )
                .ok_or_else(|| {
                    IpcResponse::error(404, "no identity rotation from this friend is pending")
                })?;
            s.facts.push(Fact::SafetyNumberChanged {
                old_safety_number,
                new_safety_number,
            });
            if matches!(request, IpcRequest::IdentityRotationAccept { .. }) {
                // The old session and verification are dropped; reversing
                // it needs the friend to rotate again.
                s.facts.push(Fact::SessionDeleted);
                s.reversibility = Reversibility::NeedsOthers;
            }
        }
        IpcRequest::IdentityMarkVerified { peer }
        | IpcRequest::IdentityClearVerification { peer } => {
            s.subject = person(ctx, peer).await?;
            let safety_number = verification::safety_number(&HostFriends::of(ctx), peer)
                .ok_or_else(|| unavailable("the safety number", "no identity is unlocked"))?;
            s.facts.push(Fact::SafetyNumber { safety_number });
            s.reversibility = Reversibility::ReversibleByInverse(
                if matches!(request, IpcRequest::IdentityMarkVerified { .. }) {
                    IntentKind::ClearVerification
                } else {
                    IntentKind::MarkVerified
                },
            );
        }
        IpcRequest::FriendAccept { public_key, .. } => {
            s.subject = person(ctx, public_key).await?;
            s.facts.push(Fact::PresenceVisible);
            s.reversibility = Reversibility::ReversibleByInverse(IntentKind::RemoveFriend);
            s.notifies = Notifies::Subject;
        }
        IpcRequest::FriendReject { public_key } => {
            s.subject = person(ctx, public_key).await?;
            s.notifies = Notifies::Subject;
        }
        IpcRequest::FriendRemove { public_key } => {
            s.subject = person(ctx, public_key).await?;
            s.facts.push(Fact::SessionDeleted);
            s.facts.push(Fact::PendingMessagesDropped);
            s.reversibility = Reversibility::NeedsOthers;
            s.notifies = Notifies::Subject;
        }
        IpcRequest::DmSend { peer_key, .. } => {
            s.subject = person(ctx, peer_key).await?;
            s.notifies = Notifies::Subject;
            s.escalations.extend(key_change(ctx, peer_key).await?);
        }
        IpcRequest::OutboxRetry { client_id } => {
            // A direct message's peer: the retry is reviewed as its send
            // was, escalated on their unacknowledged key change (R-4).
            let peer = if ctx.require_owner().is_ok() {
                dm::retry_peer(&HostFriends::of(ctx), client_id)
                    .await
                    .map_err(|e| unavailable("the send to retry", e))?
            } else {
                None
            };
            if let Some(peer) = peer {
                s.subject = person(ctx, &peer).await?;
                s.notifies = Notifies::Subject;
                s.escalations.extend(key_change(ctx, &peer).await?);
            }
        }
        IpcRequest::OutboxDiscard { .. } => {
            s.reversibility = Reversibility::Never;
        }
        _ => {}
    }
    Ok(())
}

/// `VerifiedKeyChanged` while the key we verified for `peer` changed and
/// the change is unacknowledged (`signal_identities::verified_key_changed`);
/// nothing while locked (the request then fails on its own).
async fn key_change(
    ctx: &DaemonContext,
    peer: &str,
) -> Result<Option<rekindle_types::intent::Escalation>, IpcResponse> {
    if ctx.require_owner().is_err() {
        return Ok(None);
    }
    verification::key_change(&HostFriends::of(ctx), peer)
        .await
        .map_err(|e| unavailable("the peer's key verification", e))
}

/// A person: our nickname for them first, their key fingerprint always; a
/// peer-chosen display name never alone.
pub(super) async fn person(ctx: &DaemonContext, public_key: &str) -> Result<Subject, IpcResponse> {
    let fingerprint = hex::decode(public_key)
        .map(|bytes| rekindle_crypto::identity::key_fingerprint(&bytes))
        .map_err(|e| IpcResponse::error(400, format!("public key: {e}")))?;
    let (Ok(owner), Ok(db)) = (ctx.require_owner(), ctx.require_db()) else {
        return Ok(Subject::Person {
            local_nickname: None,
            display_name: None,
            fingerprint,
        });
    };
    let pk = public_key.to_string();
    let names = db
        .call(move |conn| rekindle_db::repo::friends::names(conn, &owner, &pk))
        .await
        .map_err(|e| unavailable("the friend", e))?;
    let (local_nickname, display_name) = names.unwrap_or((None, None));
    Ok(Subject::Person {
        local_nickname,
        display_name,
        fingerprint,
    })
}

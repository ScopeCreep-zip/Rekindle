//! Who may deliver what (plan E2.3; audit G1, G9): the block list, then
//! the friend gate for a frame's kind ([`sender_admitted`]), and for a
//! PreKeyInit its own rule ([`may_open_session`]), judged before any
//! prekey is looked up. The B5 rejection of an init asks the same
//! predicate, so the two can never disagree.

use rekindle_codec::capnp_codec::direct::DmTypeId;
use rekindle_types::friend::FriendshipState;

use crate::operations::friend::{is_blocked, state, FriendHost};

/// A sender's stored friendship: `None` for a stranger, or when the row is
/// unreadable (failing closed).
pub(crate) type Status = Option<FriendshipState>;

/// Why a sender's frame goes no further than the block list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Barred {
    /// The sender is blocked: a judged refusal, answered NAK (register
    /// :76, "a NAK means … admission"; RV5 R1).
    Blocked,
    /// Our block list is unreadable: nothing was judged (fail closed, no
    /// answer).
    Unreadable,
}

/// The sender's stored friendship when they are not blocked, or why not
/// (logged): [`Barred::Blocked`], or [`Barred::Unreadable`] when the block
/// list cannot be read. The gate reads the stored row (the Host's friend
/// authority), so a frame dispatched before the projection loads is
/// judged on it.
pub(crate) async fn unblocked(host: &dyn FriendHost, sender_hex: &str) -> Result<Status, Barred> {
    match is_blocked(host, sender_hex).await {
        Ok(false) => Ok(friendship_authoritative(host, sender_hex).await),
        Ok(true) => {
            tracing::debug!(from = %sender_hex, "dropping frame from a blocked user");
            Err(Barred::Blocked)
        }
        Err(e) => {
            tracing::warn!(from = %sender_hex, error = %e, "block list unreadable; dropping frame");
            Err(Barred::Unreadable)
        }
    }
}

/// Whether `sender` may deliver a `kind` now: not blocked, and the friend
/// gate for the kind. The unlock replay asks it of every row, so a message
/// decrypted before a block or a removal is judged when it is applied.
pub(crate) async fn admitted(host: &dyn FriendHost, sender_hex: &str, kind: DmTypeId) -> bool {
    match unblocked(host, sender_hex).await {
        Ok(status) => frame_admitted(host, sender_hex, kind, status, false).await,
        Err(_) => false,
    }
}

/// Whether `sender` may deliver `kind` by the friend gate alone (not
/// blocked, and its friendship admits the kind), never the group one.
pub(crate) async fn friend_admitted(
    host: &dyn FriendHost,
    sender_hex: &str,
    kind: DmTypeId,
) -> bool {
    match unblocked(host, sender_hex).await {
        Ok(status) => kind_admitted(sender_hex, kind, status),
        Err(_) => false,
    }
}

/// Whether a sender who is not blocked, with `status`, may deliver `kind`
/// alone or as a PreKeyInit's first payload (`init`): the friend gate,
/// else the group admission (plan E2.6: a member of a group we hold, for
/// the group kinds only); logged when both refuse.
pub(crate) async fn frame_admitted(
    host: &dyn FriendHost,
    sender_hex: &str,
    kind: DmTypeId,
    status: Status,
    init: bool,
) -> bool {
    if init {
        return init_admitted(host, sender_hex, kind, status).await;
    }
    if sender_admitted(kind, status)
        || crate::operations::group_dm::admitted(host, sender_hex, kind, false).await
    {
        return true;
    }
    kind_admitted(sender_hex, kind, status)
}

/// [`sender_admitted`], logged when it refuses.
pub(crate) fn kind_admitted(sender_hex: &str, kind: DmTypeId, status: Status) -> bool {
    let admitted = sender_admitted(kind, status);
    if !admitted {
        tracing::info!(from = %sender_hex, ?status, ?kind,
            "dropping frame: sender not admitted for this kind");
    }
    admitted
}

/// Whether `sender` (not blocked), with `status`, may open a session with
/// a PreKeyInit whose first payload is `kind`: [`may_open_session`], with
/// whether a reset of ours is outstanding, else the group admission (a
/// member of a group we hold, for a pairwise group control kind only;
/// plan E2.6); logged when both refuse. The B5 rejection asks the same.
pub(crate) async fn init_admitted(
    host: &dyn FriendHost,
    sender_hex: &str,
    kind: DmTypeId,
    status: Status,
) -> bool {
    // Whether we asked for the reset is our stored request, checked and
    // consumed in the transaction that accepts the init
    // (`Reinitiation::Requested`, plan audit D-4).
    let reset_outstanding = kind == DmTypeId::SessionResetAccept;
    let admitted = may_open_session(kind, status, reset_outstanding)
        || (crate::operations::group_dm::opens_member_session(kind)
            && crate::operations::group_dm::admitted(host, sender_hex, kind, true).await);
    if !admitted {
        tracing::info!(from = %sender_hex, ?status, ?kind,
            "dropping PreKeyInit: sender may not open a session with this kind");
    }
    admitted
}

/// The sender's friendship as the identity's database holds it; `None`
/// (fail closed) when it cannot be read.
pub(crate) async fn friendship_authoritative(host: &dyn FriendHost, sender: &str) -> Status {
    let store = host.friend_store().ok()?;
    let owner = state::owner(host).ok()?;
    match store.status_of(&owner, sender).await {
        Ok(status) => status,
        Err(e) => {
            tracing::warn!(error = %e, from = %sender, "friend status unreadable; failing closed");
            None
        }
    }
}

/// Whether a sender with friendship `status` may deliver a `kind`.
///
/// - A friend request, a reject, and a removal and its ack come from
///   anyone not blocked (blocking is checked before this), and so does a
///   relay envelope: the relay forwards for a friend of the target, not of
///   ours. A `WakeNotify` is not among them: only a friend we registered
///   as our push relay wakes us (plan audit G10).
/// - `FriendAccept` (a PreKeyInit's first payload) comes from a sender we
///   sent a request to (`PendingOut`) or one already `Accepted`: the higher
///   side of a crossing request is `Accepted` with no session when the
///   lower side's accept arrives. Never from a stranger or a `Removing` row
///   (plan C7.25).
/// - Everything else needs `Accepted`.
pub(crate) fn sender_admitted(kind: DmTypeId, status: Status) -> bool {
    match kind {
        DmTypeId::FriendRequest
        | DmTypeId::FriendReject
        | DmTypeId::Unfriended
        | DmTypeId::UnfriendedAck
        | DmTypeId::RelayEnvelope => true,
        DmTypeId::FriendAccept => matches!(
            status,
            Some(FriendshipState::PendingOut | FriendshipState::Accepted)
        ),
        _ => status == Some(FriendshipState::Accepted),
    }
}

/// Whether a sender with `status` may open a session with a PreKeyInit
/// carrying `kind` (audit G1), before any prekey is looked up:
///
/// - `FriendAccept`: a sender we asked (`PendingOut`), or one already
///   `Accepted` (the higher side of a crossing request);
/// - `SessionResetAccept`: an `Accepted` friend we asked for a reset
///   (`reset_outstanding`, B11);
/// - `DmText`, `DmEdit`, `DmDelete`, `DmReaction`: an `Accepted` friend;
/// - anything else: nobody (the codec refuses those first payloads too).
///
/// The session half of the rule (an `Accepted` sender with no established
/// session, or only our own unanswered init) is the ratchet's
/// `Reinitiation::Refused`, which also refuses before any prekey lookup.
pub(crate) fn may_open_session(kind: DmTypeId, status: Status, reset_outstanding: bool) -> bool {
    let accepted = status == Some(FriendshipState::Accepted);
    match kind {
        DmTypeId::FriendAccept => matches!(
            status,
            Some(FriendshipState::PendingOut | FriendshipState::Accepted)
        ),
        DmTypeId::SessionResetAccept => accepted && reset_outstanding,
        DmTypeId::DmText | DmTypeId::DmEdit | DmTypeId::DmDelete | DmTypeId::DmReaction => accepted,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVERY_STATUS: [Status; 4] = [
        None,
        Some(FriendshipState::PendingOut),
        Some(FriendshipState::Accepted),
        Some(FriendshipState::Removing),
    ];

    #[test]
    fn friend_accept_reaches_the_requester() {
        assert!(sender_admitted(
            DmTypeId::FriendAccept,
            Some(FriendshipState::PendingOut)
        ));
        // The higher side of a crossing request is already Accepted.
        assert!(sender_admitted(
            DmTypeId::FriendAccept,
            Some(FriendshipState::Accepted)
        ));
    }

    #[test]
    fn friend_accept_from_a_stranger_or_a_removed_row_drops() {
        assert!(!sender_admitted(DmTypeId::FriendAccept, None));
        assert!(!sender_admitted(
            DmTypeId::FriendAccept,
            Some(FriendshipState::Removing)
        ));
    }

    #[test]
    fn a_pending_sender_may_send_nothing_but_the_accept() {
        for status in [
            None,
            Some(FriendshipState::PendingOut),
            Some(FriendshipState::Removing),
        ] {
            assert!(!sender_admitted(DmTypeId::DmText, status));
            assert!(!sender_admitted(DmTypeId::SessionResetRequest, status));
        }
        assert!(sender_admitted(
            DmTypeId::DmText,
            Some(FriendshipState::Accepted)
        ));
    }

    #[test]
    fn a_request_and_a_removal_come_from_anyone_not_blocked() {
        for kind in [
            DmTypeId::FriendRequest,
            DmTypeId::FriendReject,
            DmTypeId::Unfriended,
            DmTypeId::UnfriendedAck,
        ] {
            assert!(sender_admitted(kind, None));
            assert!(sender_admitted(kind, Some(FriendshipState::Removing)));
        }
    }

    #[test]
    fn a_stranger_opens_no_session_with_any_kind() {
        for kind in [
            DmTypeId::FriendAccept,
            DmTypeId::SessionResetAccept,
            DmTypeId::DmText,
            DmTypeId::DmEdit,
            DmTypeId::DmDelete,
            DmTypeId::DmReaction,
            DmTypeId::Unfriended,
            DmTypeId::UnfriendedAck,
            DmTypeId::RelayEnvelope,
            DmTypeId::FriendRequest,
        ] {
            for reset in [false, true] {
                assert!(!may_open_session(kind, None, reset), "{kind:?}");
                assert!(
                    !may_open_session(kind, Some(FriendshipState::Removing), reset),
                    "{kind:?}"
                );
            }
        }
    }

    #[test]
    fn each_first_payload_opens_a_session_only_under_its_rule() {
        for status in EVERY_STATUS {
            let accepted = status == Some(FriendshipState::Accepted);
            let asked = status == Some(FriendshipState::PendingOut);
            assert_eq!(
                may_open_session(DmTypeId::FriendAccept, status, false),
                accepted || asked
            );
            assert!(!may_open_session(
                DmTypeId::SessionResetAccept,
                status,
                false
            ));
            assert_eq!(
                may_open_session(DmTypeId::SessionResetAccept, status, true),
                accepted
            );
            assert_eq!(may_open_session(DmTypeId::DmText, status, false), accepted);
            // The removal kinds a stranger may send sealed never open one.
            assert!(!may_open_session(DmTypeId::Unfriended, status, true));
            assert!(!may_open_session(DmTypeId::RelayEnvelope, status, true));
        }
    }
}

//! `dm::handshake`'s unit tests.

use rekindle_codec::capnp_codec::direct::DmTypeId;

use super::{
    answered, awaits_answer, receipted, refuses, unfriended_client_id, unfriended_peer, Answered,
};
use crate::operations::dm::send;
use crate::operations::friend::FriendError;
use crate::operations::prekeys::Rejection;

#[test]
fn an_inbox_write_fails_only_when_refused() {
    use rekindle_records::port::PortError;
    assert!(refuses(&PortError::NotWritable("rk".into())));
    assert!(refuses(&PortError::Dht(
        "InvalidArgument: wrong writer".into()
    )));
    assert!(!refuses(&PortError::Unreachable("TryAgain".into())));
    assert!(!refuses(&PortError::Closed));
    assert!(!refuses(&PortError::Node("not attached".into())));
    assert!(!refuses(&PortError::LeaseNotHeld(3)));
}

#[test]
fn requests_first_contacts_presence_keys_and_removal_acks_wait_for_their_receipt() {
    assert!(receipted(DmTypeId::FriendRequest));
    assert!(receipted(DmTypeId::FriendAccept));
    assert!(receipted(DmTypeId::SessionResetAccept));
    assert!(!receipted(DmTypeId::Unfriended));
    assert!(receipted(DmTypeId::FriendsKey));
    assert!(receipted(DmTypeId::GroupDmAccept));
    assert!(receipted(DmTypeId::GroupDmInvite));
    assert!(receipted(DmTypeId::GroupDmRosterGrow));
    assert!(receipted(DmTypeId::UnfriendedAck));
}

#[test]
fn the_unfriended_row_names_its_peer() {
    let id = unfriended_client_id("ab12");
    assert_eq!(unfriended_peer(&id), Some("ab12"));
    assert_eq!(unfriended_peer("FriendRequest:00"), None);
}

#[test]
fn a_reply_that_did_not_open_or_was_not_applied_is_a_refusal_never_a_receipt() {
    let reply = || Ok(send::CallAnswer::Reply(b"garbage".to_vec()));
    for kind in [
        DmTypeId::FriendAccept,
        DmTypeId::SessionResetAccept,
        DmTypeId::FriendsKey,
    ] {
        assert_eq!(
            answered(kind, &reply(), Rejection::NotOurs),
            Answered::Refused
        );
        assert_eq!(answered(kind, &reply(), Rejection::Failed), Answered::Again);
        assert_eq!(
            answered(kind, &reply(), Rejection::Applied),
            Answered::Rejected
        );
        assert_eq!(
            answered(kind, &Ok(send::CallAnswer::Refused), Rejection::NotOurs),
            Answered::Refused
        );
        assert_eq!(
            answered(kind, &Ok(send::CallAnswer::Receipt), Rejection::NotOurs),
            Answered::Receipt
        );
        assert_eq!(
            answered(kind, &Err(FriendError::Locked), Rejection::NotOurs),
            Answered::Again
        );
    }
    assert_eq!(
        answered(DmTypeId::FriendRequest, &reply(), Rejection::NotOurs),
        Answered::Again
    );
}

/// LV23 review M1: a friend request's bare NAK is never a block (that
/// is the silent receipt) and never ends the row: attempted again.
#[test]
fn a_nakd_friend_request_is_attempted_again_never_refused() {
    assert_eq!(
        answered(
            DmTypeId::FriendRequest,
            &Ok(send::CallAnswer::Refused),
            Rejection::NotOurs
        ),
        Answered::Undelivered
    );
}

/// Review RV5 M2: their "retry later" is their own fault, attempted
/// again as a timeout is, whatever the kind.
#[test]
fn a_retry_later_is_attempted_again_for_every_kind() {
    for kind in [
        DmTypeId::FriendRequest,
        DmTypeId::FriendAccept,
        DmTypeId::SessionResetAccept,
        DmTypeId::UnfriendedAck,
    ] {
        for rejection in [Rejection::Applied, Rejection::Failed, Rejection::NotOurs] {
            assert_eq!(
                answered(kind, &Ok(send::CallAnswer::RetryLater), rejection),
                Answered::Again
            );
        }
    }
}

/// Plan CE3: a `CallInvite` waits for its answer like an `Unfriended`;
/// other route frames settle once they leave.
#[test]
fn a_call_invite_awaits_its_answer() {
    assert!(awaits_answer(DmTypeId::CallInvite));
    assert!(awaits_answer(DmTypeId::Unfriended));
    assert!(!awaits_answer(DmTypeId::CallEnd));
    assert!(!awaits_answer(DmTypeId::CallRinging));
}

/// Plan CE4: a `CallEnd` goes by `app_call` and settles only on its
/// receipt (re-sent until then or its deadline); a `CallRinging` does not.
#[test]
fn a_call_end_awaits_its_receipt() {
    assert!(receipted(DmTypeId::CallEnd));
    assert!(!receipted(DmTypeId::CallRinging));
    assert!(!receipted(DmTypeId::CallInvite));
}

#[test]
fn reset_requests_and_declines_wait_for_their_receipt() {
    for kind in [DmTypeId::SessionResetRequest, DmTypeId::SessionResetDecline] {
        assert!(receipted(kind));
        assert!(!awaits_answer(kind));
        for (answer, expected) in [
            (Ok(send::CallAnswer::Receipt), Answered::Receipt),
            (Ok(send::CallAnswer::Refused), Answered::Refused),
            (Ok(send::CallAnswer::RetryLater), Answered::Again),
            (
                Err(FriendError::Failed("app_call timed out".into())),
                Answered::Again,
            ),
        ] {
            assert_eq!(answered(kind, &answer, Rejection::NotOurs), expected);
        }
    }
}

#[test]
fn co_creator_answers_withdrawals_and_join_links_wait_for_their_receipt() {
    for kind in [
        DmTypeId::CreatorAccept,
        DmTypeId::CreatorDecline,
        DmTypeId::CreatorWithdrawn,
        DmTypeId::CreatorJoin,
    ] {
        assert!(receipted(kind));
    }
}

use super::handle_leave;
use parking_lot::RwLock;
use rekindle_transport::payload::rpc::CommunityLeaveNotification;
use rekindle_transport::session::{CommunityMembership, Session, SessionIdentity};

const GOV: &str = "VLD0:gov:key";
const DEPARTED: &str = "ff00";

fn identity() -> SessionIdentity {
    SessionIdentity {
        public_key_hex: "ab".repeat(32),
        display_name: "tester".into(),
        profile_dht_key: String::new(),
        mailbox_dht_key: String::new(),
        friend_list_dht_key: String::new(),
        profile_owner_keypair: None,
        friend_list_owner_keypair: None,
    }
}

/// A *non-operator* membership on purpose: under v2.0 the response
/// to a departure must not depend on being privileged, which the
/// v1.0 handler gated on.
fn membership() -> CommunityMembership {
    CommunityMembership {
        anchor: GOV.into(),
        pseudonym_key: "cd".repeat(32),
        registry_key: "VLD0:reg:key".into(),
        slot_index: 3,
        community_name: "test".into(),
        slot_seed: None,
        segment_index: Some(0),
        mek_generation: 1,
    }
}

fn session_with_community() -> RwLock<Option<Session>> {
    let mut session = Session::new(identity());
    session.join_community(membership());
    RwLock::new(Some(session))
}

fn notification() -> CommunityLeaveNotification {
    CommunityLeaveNotification {
        anchor: GOV.into(),
        leaving_pseudonym_hex: DEPARTED.into(),
    }
}

/// The whole v2.0 contract for this handler: a departure queues a
/// rotation and does nothing else. No operator check, no member
/// index rewrite, no MEK vault write.
#[test]
fn departure_queues_a_rotation_for_a_plain_member() {
    let (tx, mut rx) = crate::daemon::mek_rotation::channel();
    handle_leave(&notification(), &session_with_community(), &tx);

    let queued = rx.try_recv().expect("a rotation should have been queued");
    assert_eq!(queued.community_id, GOV);
    match queued.kind {
        crate::daemon::mek_rotation::MekRotationKind::Departure {
            departed_pseudonym_hex,
        } => assert_eq!(departed_pseudonym_hex, DEPARTED),
        other => panic!("expected a departure rotation, got {other:?}"),
    }
    assert!(rx.try_recv().is_err(), "exactly one request");
}

/// An unsolicited notification for a community we never joined must
/// not create work. Anyone who can reach our route can send one.
#[test]
fn departure_for_an_unknown_community_is_ignored() {
    let (tx, mut rx) = crate::daemon::mek_rotation::channel();
    let mut notif = notification();
    notif.anchor = "VLD0:some:other".into();

    handle_leave(&notif, &session_with_community(), &tx);
    assert!(rx.try_recv().is_err(), "must not queue for a stranger");
}

/// No session at all (daemon locked) is the same answer.
#[test]
fn departure_with_no_session_is_ignored() {
    let (tx, mut rx) = crate::daemon::mek_rotation::channel();
    handle_leave(&notification(), &RwLock::new(None), &tx);
    assert!(rx.try_recv().is_err());
}

/// A dead worker must not take the handler down with it — the
/// leaving peer is already gone and cannot act on a failure.
#[test]
fn a_dropped_worker_does_not_panic_the_handler() {
    let (tx, rx) = crate::daemon::mek_rotation::channel();
    drop(rx);
    handle_leave(&notification(), &session_with_community(), &tx);
}

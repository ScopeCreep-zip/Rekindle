//! The pre-session open (`frame::open_presession`).

use super::*;

fn presession(
    from: &Identity,
    to: &Identity,
    sent_at: u64,
) -> (DmTypeId, rekindle_codec::capnp_codec::direct::SealedBody) {
    let body = DirectBody::FriendReject;
    let frame = crate::operations::dm::frame::seal_presession(
        from,
        &to.public_key_bytes(),
        &body,
        [7; 16],
        sent_at,
    )
    .unwrap();
    let FrameBody::Sealed(sealed) = frame.body else {
        panic!("a sealed kind travels sealed");
    };
    assert_eq!(frame.sender, from.public_key_bytes());
    (frame.type_id, sealed)
}

#[test]
fn a_sealed_frame_opens_to_its_recipient() {
    let (alice, bob) = (Identity::generate(), Identity::generate());
    let (kind, sealed) = presession(&alice, &bob, 1_000);
    let content = open_presession(&bob, &alice.public_key_bytes(), kind, &sealed, 1_000).unwrap();
    assert_eq!(content.nonce, [7; 16]);
    assert_eq!(content.sent_at_ms, 1_000);
}

#[test]
fn a_forged_sender_is_refused() {
    // Mallory seals a valid HPKE frame to Bob and claims Alice sent it.
    let (alice, bob, mallory) = (
        Identity::generate(),
        Identity::generate(),
        Identity::generate(),
    );
    let (kind, sealed) = presession(&mallory, &bob, 1_000);
    assert_eq!(
        open_presession(&bob, &alice.public_key_bytes(), kind, &sealed, 1_000),
        Err("bad signature")
    );
}

#[test]
fn a_forged_session_reset_request_is_refused() {
    // Mallory asks Bob to reset his session with Alice, in Alice's name.
    let (alice, bob, mallory) = (
        Identity::generate(),
        Identity::generate(),
        Identity::generate(),
    );
    let body =
        DirectBody::SessionResetRequest(rekindle_codec::capnp_codec::direct::SessionResetRequest {
            reason: String::new(),
        });
    let frame = crate::operations::dm::frame::seal_presession(
        &mallory,
        &bob.public_key_bytes(),
        &body,
        [5; 16],
        1_000,
    )
    .unwrap();
    let FrameBody::Sealed(sealed) = frame.body else {
        panic!("a sealed kind travels sealed");
    };
    assert_eq!(
        open_presession(
            &bob,
            &alice.public_key_bytes(),
            frame.type_id,
            &sealed,
            1_000
        ),
        Err("bad signature")
    );
}

#[test]
fn a_frame_readdressed_to_someone_else_is_refused() {
    let (alice, bob, carol) = (
        Identity::generate(),
        Identity::generate(),
        Identity::generate(),
    );
    let (kind, sealed) = presession(&alice, &bob, 1_000);
    assert_eq!(
        open_presession(&carol, &alice.public_key_bytes(), kind, &sealed, 1_000),
        Err("bad signature")
    );
}

#[test]
fn a_relabelled_or_stale_frame_is_refused() {
    let (alice, bob) = (Identity::generate(), Identity::generate());
    let (_, sealed) = presession(&alice, &bob, 1_000);
    let from = alice.public_key_bytes();
    assert!(open_presession(&bob, &from, DmTypeId::SessionResetDecline, &sealed, 1_000).is_err());
    assert_eq!(
        open_presession(
            &bob,
            &from,
            DmTypeId::FriendReject,
            &sealed,
            1_000 + ENVELOPE_FRESHNESS_WINDOW_MS + 1
        ),
        Err("stale")
    );
}

#[test]
fn freshness_allows_skew_ahead_and_the_window_behind() {
    let now = 10_000_000;
    assert!(fresh(now, now));
    assert!(fresh(now + ENVELOPE_MAX_FUTURE_SKEW_MS, now));
    assert!(!fresh(now + ENVELOPE_MAX_FUTURE_SKEW_MS + 1, now));
    assert!(fresh(now - ENVELOPE_FRESHNESS_WINDOW_MS, now));
    assert!(!fresh(now - ENVELOPE_FRESHNESS_WINDOW_MS - 1, now));
}

/// RV5 R2: our database failing in a ratchet step judged nothing (no
/// answer); any other failure is the frame's (NAK).
#[test]
fn a_ratchet_db_error_is_ours_and_any_other_failure_is_a_refusal() {
    let db = RatchetError::Db(rekindle_asql::Error::ConnectionClosed);
    assert!(matches!(ratchet_dropped(&db), Dropped::Failed));
    let theirs = RatchetError::EstablishedSession;
    assert!(matches!(ratchet_dropped(&theirs), Dropped::Refused));
}

//! Opening an inbound frame (`frame`).

use super::*;

/// RV5 R2: a ratchet failure other than our database is the frame's (NAK).
#[test]
fn a_ratchet_failure_of_the_frame_is_a_refusal() {
    let theirs = RatchetError::EstablishedSession;
    assert!(matches!(ratchet_dropped(&theirs), Dropped::Refused));
}

use super::JoinRefusal;

#[test]
fn every_reason_round_trips_its_wire_byte() {
    for r in [
        JoinRefusal::InviteUsedUp,
        JoinRefusal::InviteNoLongerValid,
        JoinRefusal::InviteUnknown,
        JoinRefusal::NotAdmitted,
    ] {
        assert_eq!(JoinRefusal::from_wire(r.to_wire()), r);
        assert!((400..500).contains(&r.code()));
    }
}

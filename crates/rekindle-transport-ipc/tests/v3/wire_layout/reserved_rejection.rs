use rekindle_transport_ipc::v3::wire::lane::Lane;

#[test]
fn lane_zero_is_rejected() {
    assert!(Lane::try_from(0x00).is_err());
}

#[test]
fn lane_0xff_is_rejected() {
    assert!(Lane::try_from(0xFF).is_err());
}

#[test]
fn lanes_0x05_through_0xfe_are_all_rejected() {
    for v in 0x05..=0xFE {
        assert!(
            Lane::try_from(v).is_err(),
            "Lane should reject reserved value 0x{v:02x}"
        );
    }
}

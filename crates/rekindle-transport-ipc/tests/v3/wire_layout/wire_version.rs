use rekindle_transport_ipc::v3::wire::constants::WIRE_VERSION;

#[test]
fn wire_version_is_v1() {
    assert_eq!(WIRE_VERSION, 0x01);
}

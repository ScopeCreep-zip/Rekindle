use rekindle_transport_ipc::v3::wire::lane::Lane;

#[test]
fn control_is_0x01() { assert_eq!(Lane::Control as u8, 0x01); }

#[test]
fn data_is_0x02() { assert_eq!(Lane::Data as u8, 0x02); }

#[test]
fn audit_is_0x03() { assert_eq!(Lane::Audit as u8, 0x03); }

#[test]
fn handoff_is_0x04() { assert_eq!(Lane::Handoff as u8, 0x04); }

#[test]
fn all_variants_has_four_entries() {
    assert_eq!(Lane::ALL.len(), 4);
}

#[test]
fn all_variants_priority_order_control_first_data_last() {
    assert_eq!(Lane::ALL[0], Lane::Control);
    assert_eq!(Lane::ALL[3], Lane::Data);
}

use rekindle_transport_ipc::v3::wire::frame_class::FrameClass;

#[test]
fn channel_is_0x01() { assert_eq!(FrameClass::Channel as u8, 0x01); }

#[test]
fn stream_is_0x02() { assert_eq!(FrameClass::Stream as u8, 0x02); }

#[test]
fn datagram_is_0x03() { assert_eq!(FrameClass::Datagram as u8, 0x03); }

#[test]
fn audit_is_0x04() { assert_eq!(FrameClass::Audit as u8, 0x04); }

#[test]
fn handoff_is_0x05() { assert_eq!(FrameClass::Handoff as u8, 0x05); }

#[test]
fn all_variants_has_five_entries() {
    assert_eq!(FrameClass::ALL.len(), 5);
}

#[test]
fn zero_is_not_a_valid_frame_class() {
    assert!(FrameClass::try_from(0x00).is_err());
}

#[test]
fn values_above_0x05_are_not_valid() {
    for v in 0x06..=0xFF {
        assert!(
            FrameClass::try_from(v).is_err(),
            "FrameClass should reject 0x{v:02x}"
        );
    }
}

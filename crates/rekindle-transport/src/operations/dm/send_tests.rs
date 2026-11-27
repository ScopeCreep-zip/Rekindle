use super::*;

#[test]
fn a_stored_frame_survives_its_json_row() {
    for queued in [
        Framed::Wire {
            datagram: vec![b'D', 1, 2, 3],
        },
        Framed::Sealed {
            type_id: DmTypeId::SessionResetDecline.as_u16(),
            body: Vec::new(),
            nonce: [9; 16],
        },
        Framed::Pending {
            type_id: DmTypeId::GroupDmAccept.as_u16(),
            body: vec![4, 5, 6],
        },
    ] {
        let row = serde_json::to_string(&queued).unwrap();
        assert_eq!(serde_json::from_str::<Framed>(&row).unwrap(), queued);
    }
}

#[test]
fn hints_follow_the_kind() {
    assert_eq!(hint_for(DmTypeId::DmText), ContentHint::Resendable);
    assert_eq!(hint_for(DmTypeId::Typing), ContentHint::Implicit);
    assert_eq!(hint_for(DmTypeId::CallInvite), ContentHint::Default);
}

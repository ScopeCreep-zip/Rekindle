#[test]
fn a_row_payload_round_trips() {
    let payload = super::encode("VLD0:a", "VLD0:m", &[1, 2, 3]).unwrap();
    assert_eq!(
        super::decode(&payload),
        Some(super::RowBody::Op {
            community: "VLD0:a",
            record: "VLD0:m",
            op: &[1u8, 2, 3][..],
        })
    );
}

#[test]
fn a_revoke_row_payload_round_trips() {
    let payload = super::encode_revoke("VLD0:a", [7; 16]).unwrap();
    assert_eq!(
        super::decode(&payload),
        Some(super::RowBody::RevokeUnlessLanded {
            community: "VLD0:a",
            invite_id: [7; 16],
        })
    );
    assert!(!super::is_op_row(&super::revoke_client_id([7; 16])));
}

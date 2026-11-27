use super::*;
use rekindle_utils::random::OsRandom;

const AT: MemberRecordAad<'static> = MemberRecordAad {
    community_id: "community",
    member_record: "VLD0:member",
};

#[test]
fn round_trip_and_binding() {
    let key = MediaEncryptionKey::from_bytes([7u8; 32], 3);
    let sealed = seal_member_record_field(&key, AT, b"ranges", &OsRandom).unwrap();
    assert_eq!(
        open_member_record_field(&key, AT, &sealed).unwrap(),
        b"ranges"
    );
    let other_record = MemberRecordAad {
        member_record: "VLD0:other",
        ..AT
    };
    assert!(open_member_record_field(&key, other_record, &sealed).is_err());
    // The boundary between the two parts is part of the AAD.
    let shifted = MemberRecordAad {
        community_id: "communityV",
        member_record: "LD0:member",
    };
    assert!(open_member_record_field(&key, shifted, &sealed).is_err());
    let rotated = MediaEncryptionKey::from_bytes([8u8; 32], 4);
    assert!(open_member_record_field(&rotated, AT, &sealed).is_err());
}

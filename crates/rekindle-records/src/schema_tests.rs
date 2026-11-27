use super::*;

#[test]
fn layouts_size_their_records() {
    // Nine subkeys and the friend code's CODE_INBOX (plan RV2e).
    assert_eq!(profile().subkey_count(), 10);
    assert_eq!(mailbox().subkey_count(), 1);
    assert_eq!(prekeys().subkey_count(), 101);
    assert_eq!(request_inbox(&[3u8; 32]).subkey_count(), 2);
    assert_eq!(community_anchor().subkey_count(), 1);
    assert_eq!(member_record().subkey_count(), 8);
    assert_eq!(sealed_page().subkey_count(), 1);
    let keys = [[1u8; 32], [2u8; 32]];
    assert_eq!(community_smpl(&keys).subkey_count(), 2);
    assert_eq!(channel(&keys).subkey_count(), 2);
}

/// Decision 67 item 3: 65 subkeys, and writer `i`'s Veilid range (owner
/// subkeys first, then each member's `m_cnt` in order, derivation.rs
/// `check_subkey_value_data_smpl`) is exactly row `i` of the layout.
#[test]
fn group_rendezvous_rows_are_their_writers_ranges() {
    use layout::group_rendezvous as g;
    let writers: [[u8; 32]; 8] = std::array::from_fn(|i| [u8::try_from(i).unwrap() + 1; 32]);
    let schema = group_rendezvous(&writers);
    assert_eq!(schema.subkey_count(), 65);
    assert_eq!(usize::from(g::SUBKEY_COUNT), 65);
    assert_eq!(g::MAX_SUBKEY_VALUE_LEN, 1_048_576 / 65);
    let RecordSchema::Smpl {
        owner_subkeys,
        members,
    } = &schema
    else {
        panic!("a rendezvous record is SMPL");
    };
    let mut start = u32::from(*owner_subkeys);
    for (i, member) in members.iter().enumerate() {
        let row = u8::try_from(i).unwrap();
        assert_eq!(member.key, writers[i]);
        let range = start..start + u32::from(member.subkeys);
        for col in (0..g::WRITERS).filter(|c| *c != row) {
            let subkey = g::subkey(row, col).unwrap();
            assert!(range.contains(&subkey), "row {row} col {col}");
            assert_eq!(g::slots_of(subkey), Some((row, col)));
        }
        start = range.end;
    }
    assert_eq!(g::subkey(2, 2), None);
    assert_eq!(g::subkey(8, 0), None);
    assert_eq!(g::slots_of(g::OWNER_SUBKEY), None);
    assert_eq!(g::column(3).len(), 7);
    assert!(g::column(3)
        .iter()
        .all(|s| g::slots_of(*s).map(|(_, c)| c) == Some(3)));
}

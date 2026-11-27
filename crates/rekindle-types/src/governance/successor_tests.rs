use super::*;

fn successor(invites: Option<&str>) -> CommunitySuccessor {
    CommunitySuccessor {
        new_anchor: Some("VLD0:new".into()),
        reason: SuccessorReason::Recreate,
        transfer_to: None,
        creator_set_hash: None,
        accepts: None,
        links: Vec::new(),
        invites_record: invites.map(str::to_owned),
    }
}

#[test]
fn a_successor_carries_its_invites_record_and_nothing_else_does() {
    assert_eq!(
        successor(Some("VLD0:inv")).shape(),
        Some(SuccessorShape::Successor)
    );
    assert_eq!(successor(None).shape(), None);
    assert_eq!(successor(Some("")).shape(), None);
    let withdrawal = CommunitySuccessor {
        new_anchor: None,
        reason: SuccessorReason::Transfer,
        ..successor(Some("VLD0:inv"))
    };
    assert_eq!(withdrawal.shape(), None);
}

/// Plan E3.2 S7s.4: an acceptance names the new community the members
/// follow, so it needs the sealed invites like a successor does.
#[test]
fn a_transfer_acceptance_carries_its_invites_record() {
    let acceptance = |invites: Option<&str>| CommunitySuccessor {
        reason: SuccessorReason::Transfer,
        creator_set_hash: Some([1; 32]),
        accepts: Some(OpRef {
            op_hash: super::super::OpHash([2; 32]),
            device_key: [3; 32],
            author: PseudonymKey([3; 32]),
            author_record: "VLD0:rec".into(),
            author_seq: 1,
        }),
        links: vec![CreatorLink {
            old: PseudonymKey([4; 32]),
            new: PseudonymKey([5; 32]),
            sig: vec![0; 64],
        }],
        ..successor(invites)
    };
    assert_eq!(
        acceptance(Some("VLD0:inv")).shape(),
        Some(SuccessorShape::Acceptance)
    );
    assert_eq!(acceptance(None).shape(), None);
}

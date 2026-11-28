use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
use rekindle_types::governance::{AdmissionMode, OpRef};

use super::*;
use crate::checkpoint::directory::{CheckpointDirectory, DirLeafV1, MemberRecordKey};
use crate::checkpoint::{eligible_set_hash, frontier_hash};
use crate::fixture::{cert, meta, op};

const ANCHOR: &str = "VLD0:anchor";

fn key(i: u8) -> SigningKey {
    SigningKey::from_bytes(&[i; 32])
}

fn pk(i: u8) -> PseudonymKey {
    PseudonymKey(key(i).verifying_key().to_bytes())
}

fn body(height: u64, prev: Option<OpRef>, electorate: &[PseudonymKey]) -> Checkpoint {
    let head = op(&pk(1), 0, &[], 1, meta("C"));
    Checkpoint {
        height,
        round: 0,
        prev,
        cut: vec![head.op_ref("VLD0:creator-record")],
        frontier_hash: frontier_hash(&[head.hash()]),
        dir_root: [0; 32],
        dir_size: 0,
        state_root: [0; 32],
        state_size: 0,
        mek_generation: 1,
        eligible_set_hash: eligible_set_hash(electorate),
        pages: Vec::new(),
    }
}

/// `signer`'s `Checkpoint` op carrying `body`, signed over stand-in
/// unsigned bytes (the signature covers exactly those bytes), verified.
fn endorse(signer: u8, seq: u64, body: &Checkpoint) -> SignedEndorsement {
    let mut op = GovernanceOp {
        device_key: pk(signer).0,
        author: pk(signer),
        seq,
        prev: None,
        deps: body.cut.clone(),
        issued_at: 1,
        body: GovernanceEntry::Checkpoint(body.clone()),
        sig: [0; 64],
    };
    let unsigned = [&[signer], seq.to_be_bytes().as_slice()].concat();
    op.sig = key(signer)
        .sign(&op_signing_bytes(ANCHOR, &unsigned))
        .to_bytes();
    SignedEndorsement::verify(ANCHOR, &op, &unsigned).expect("a well-signed endorsement")
}

/// The state at the height-1 cut: members 1–3 joined.
fn members_1_to_3() -> GovernanceState {
    let mut state = GovernanceState::default();
    for i in 1..=3 {
        state.members.insert(
            pk(i),
            crate::state::MemberState {
                member_record: format!("VLD0:m{i}"),
                member_record_owner: String::new(),
                joined: crate::op::OpStamp::MIN,
            },
        );
    }
    state
}

/// Plan E3.2 S7c.5: the height-1 hint is endorsed by the creator set, and
/// the height-2 hint by a strict majority of the members at the height-1
/// cut whose endorsements name a height-1 endorsement.
#[test]
fn a_hint_chain_is_checked_down_to_the_genesis_cert() {
    let cert = cert(&pk(1), AdmissionMode::Open);
    let one = endorse(1, 1, &body(1, None, &[pk(1)]));
    let link1 = endorsed_link(&cert, ANCHOR, None, std::slice::from_ref(&one)).expect("height 1");
    let at_cut = members_1_to_3();
    let electorate = [pk(1), pk(2), pk(3)];
    let prev = OpRef {
        op_hash: one.hash,
        device_key: pk(1).0,
        author: pk(1),
        author_record: "VLD0:creator-record".into(),
        author_seq: 1,
    };
    let two = body(2, Some(prev), &electorate);
    let votes = [endorse(1, 2, &two), endorse(3, 0, &two)];
    let link2 = endorsed_link(&cert, ANCHOR, Some((&link1, &at_cut)), &votes).expect("height 2");
    assert_eq!((link2.height, link2.body.clone()), (2, two.clone()));

    // One endorser of three is no majority.
    assert!(endorsed_link(&cert, ANCHOR, Some((&link1, &at_cut)), &votes[..1]).is_none());
    // A height-1 hint endorsed by a non-creator does not chain to the cert.
    let forged = endorse(2, 0, &body(1, None, &[pk(2)]));
    assert!(endorsed_link(&cert, ANCHOR, None, &[forged]).is_none());
    // Height-2 endorsements naming no height-1 endorsement do not chain.
    let orphan = body(
        2,
        Some(OpRef {
            op_hash: OpHash([5; 32]),
            ..votes[0].body.prev.clone().unwrap()
        }),
        &electorate,
    );
    let orphans = [endorse(1, 2, &orphan), endorse(3, 0, &orphan)];
    assert!(endorsed_link(&cert, ANCHOR, Some((&link1, &at_cut)), &orphans).is_none());
    // Endorsers outside the members at the height-1 cut count for nothing.
    let strangers = [endorse(7, 0, &two), endorse(8, 0, &two)];
    assert!(endorsed_link(&cert, ANCHOR, Some((&link1, &at_cut)), &strangers).is_none());
}

/// A bad signature is no endorsement.
#[test]
fn an_endorsement_whose_signature_fails_is_refused() {
    let e = endorse(1, 1, &body(1, None, &[pk(1)]));
    let op = GovernanceOp {
        device_key: pk(1).0,
        author: pk(1),
        seq: 1,
        prev: None,
        deps: e.body.cut.clone(),
        issued_at: 1,
        body: GovernanceEntry::Checkpoint(e.body.clone()),
        sig: [9; 64],
    };
    assert!(SignedEndorsement::verify(ANCHOR, &op, b"x").is_none());
}

/// The pages a body names are checked against its `dirRoot`: the sealed
/// pages pass, a page from another directory fails.
#[test]
fn directory_pages_are_checked_against_the_dir_root() {
    let community = [4; 32];
    let leaf = |i: u8| DirLeafV1 {
        community_id: community,
        member: pk(i),
        member_record: MemberRecordKey {
            kind: *b"VLD0",
            key: [i; 32],
            encryption_key: [i; 32],
        },
        member_record_owner: [i; 32],
        join_op_hash: OpHash([i; 32]),
        join_height: 1,
    };
    let dir = CheckpointDirectory::new((1..=3).map(leaf).collect());
    let mut b = body(1, None, &[pk(1)]);
    b.dir_root = dir.root().0;
    b.dir_size = dir.size();
    b.pages = vec!["VLD0:page".into()];
    let pages: Vec<DirectoryPage> = (0..dir.page_count())
        .filter_map(|i| dir.page(community, 1, i))
        .collect();
    assert!(pages_match(&community, &b, &pages));
    let other = CheckpointDirectory::new((4..=6).map(leaf).collect());
    let wrong: Vec<DirectoryPage> = (0..other.page_count())
        .filter_map(|i| other.page(community, 1, i))
        .collect();
    assert!(!pages_match(&community, &b, &wrong));
    assert!(!pages_match(&community, &b, &[]));
}

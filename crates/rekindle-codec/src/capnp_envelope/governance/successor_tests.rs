use rekindle_types::governance::{
    CommunitySuccessor, CreatorLink, GovernanceEntry, OpHash, OpRef, SuccessorReason,
};
use rekindle_types::id::PseudonymKey;

use super::super::{decode_governance_entry, encode_governance_entry};

/// The parts a successor body may carry, each present or not.
#[derive(Default)]
struct Parts {
    anchor: Option<String>,
    to: Option<PseudonymKey>,
    set: Option<[u8; 32]>,
    accepts: Option<OpRef>,
    links: u8,
    invites: Option<String>,
}

fn anchor() -> String {
    format!("VLD0:{}", "a".repeat(43))
}

fn accepted() -> OpRef {
    OpRef {
        op_hash: OpHash([9; 32]),
        device_key: [10; 32],
        author: PseudonymKey([10; 32]),
        author_record: format!("VLD0:{}", "r".repeat(43)),
        author_seq: 3,
    }
}

fn body(parts: Parts) -> GovernanceEntry {
    let reason = if parts.to.is_some() || parts.set.is_some() {
        SuccessorReason::Transfer
    } else {
        SuccessorReason::Recreate
    };
    GovernanceEntry::CommunitySuccessor(CommunitySuccessor {
        new_anchor: parts.anchor,
        reason,
        transfer_to: parts.to,
        creator_set_hash: parts.set,
        accepts: parts.accepts,
        invites_record: parts.invites,
        links: (0..parts.links)
            .map(|i| CreatorLink {
                old: PseudonymKey([i; 32]),
                new: PseudonymKey([i + 100; 32]),
                sig: vec![i; 64],
            })
            .collect(),
    })
}

/// A recreate successor naming its new anchor.
fn successor() -> GovernanceEntry {
    body(Parts {
        anchor: Some(anchor()),
        invites: Some(format!("VLD0:{}", "i".repeat(43))),
        ..Parts::default()
    })
}

/// A transfer offer to one member, over the current creator set.
fn offer() -> GovernanceEntry {
    body(Parts {
        to: Some(PseudonymKey([7; 32])),
        set: Some([8; 32]),
        ..Parts::default()
    })
}

/// A withdrawal: no part at all.
fn withdrawal() -> GovernanceEntry {
    body(Parts::default())
}

/// An acceptance of an offer, with its new anchor and `links` links.
fn acceptance(links: u8) -> GovernanceEntry {
    body(Parts {
        anchor: Some(anchor()),
        set: Some([8; 32]),
        accepts: Some(accepted()),
        links,
        invites: Some(format!("VLD0:{}", "j".repeat(43))),
        ..Parts::default()
    })
}

#[test]
fn community_successor_round_trips() {
    let all = [successor(), offer(), withdrawal(), acceptance(9)];
    for entry in all {
        let bytes = encode_governance_entry(&entry);
        assert_eq!(decode_governance_entry(&bytes.unwrap()).expect("decodes"), entry);
    }
}

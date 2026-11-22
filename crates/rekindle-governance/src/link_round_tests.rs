//! Lead decision (46): the link round's fit rules.

use rekindle_types::governance::{
    CommunitySuccessor, CreatorLink, OpHash, OpRef, SuccessorLink, SuccessorLinkRequest,
    SuccessorReason,
};
use rekindle_types::id::PseudonymKey;

use super::*;

fn key(b: u8) -> PseudonymKey {
    PseudonymKey([b; 32])
}

fn op_ref(b: u8) -> OpRef {
    OpRef {
        op_hash: OpHash([b; 32]),
        author: key(b),
        author_record: "VLD0:rec".into(),
        author_seq: 1,
    }
}

fn offer(to: u8, set: &[u8]) -> CommunitySuccessor {
    let set: Vec<PseudonymKey> = set.iter().map(|b| key(*b)).collect();
    CommunitySuccessor {
        new_anchor: None,
        reason: SuccessorReason::Transfer,
        transfer_to: Some(key(to)),
        creator_set_hash: Some(successor_set_hash(set.iter())),
        accepts: None,
        links: Vec::new(),
        invites_record: None,
    }
}

fn request(asked: &[u8]) -> SuccessorLinkRequest {
    SuccessorLinkRequest {
        offer: op_ref(9),
        new_anchor: "VLD0:new".into(),
        asked: asked.iter().map(|b| key(*b)).collect(),
    }
}

/// Only the offer's recipient asks, and it asks exactly the offer's other
/// creators.
#[test]
fn a_request_fits_only_its_offer_to_its_author() {
    let offer = offer(3, &[2, 3]);
    assert!(requests_offer(&key(3), &request(&[2]), &offer));
    assert!(!requests_offer(&key(4), &request(&[2]), &offer));
    assert!(!requests_offer(&key(3), &request(&[2, 5]), &offer));
    assert!(!requests_offer(&key(3), &request(&[2, 3]), &offer));
}

/// A link answers a request only when its author is asked and signs its
/// own link.
#[test]
fn a_link_answers_only_a_request_asking_its_author() {
    let link = |old: u8| SuccessorLink {
        request: op_ref(7),
        link: CreatorLink {
            old: key(old),
            new: key(40),
            sig: vec![0; 64],
        },
        creator_sig: vec![0; 64],
    };
    assert!(answers_request(&key(2), &link(2), &request(&[2])));
    assert!(!answers_request(&key(5), &link(5), &request(&[2])));
    assert!(!answers_request(&key(2), &link(5), &request(&[2, 5])));
}

/// One link per creator per request: a second one by merge order is
/// dropped.
#[test]
fn a_creators_second_link_to_a_request_is_dropped() {
    let mut state = GovernanceState::default();
    let stamp = |b: u8| OpStamp {
        hash: OpHash([b; 32]),
        lamport: u64::from(b),
    };
    let link = |sig: u8| {
        GovernanceEntry::SuccessorLink(SuccessorLink {
            request: op_ref(7),
            link: CreatorLink {
                old: key(2),
                new: key(40),
                sig: vec![sig; 64],
            },
            creator_sig: vec![0; 64],
        })
    };
    apply(&key(2), &link(1), stamp(11), &mut state);
    apply(&key(2), &link(2), stamp(12), &mut state);
    let held: Vec<_> = links_to(&state, &OpHash([7; 32])).collect();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].2.link.sig, vec![1; 64]);
}

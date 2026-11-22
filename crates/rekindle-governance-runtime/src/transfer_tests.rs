//! Plan E3.2 S7s.4: the transfer's offer, withdrawal, round and links
//! (lead decisions (24)–(26)).

use rekindle_governance::op::OpStamp;
use rekindle_governance::state::MemberState;
use rekindle_governance::successor::link_signing_bytes;
use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};

use super::*;

const OLD: &str = "VLD0:old";
const NEW: &str = "VLD0:new";

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn pk(seed: u8) -> PseudonymKey {
    PseudonymKey(key(seed).verifying_key().to_bytes())
}

/// Creators 1 and 3, members 1–4.
fn state() -> GovernanceState {
    let mut s = GovernanceState {
        creators: [pk(1), pk(3)].into_iter().collect(),
        ..GovernanceState::default()
    };
    for m in 1..=4 {
        let joined = OpStamp {
            lamport: 1,
            hash: OpHash([m; 32]),
        };
        let row = MemberState {
            member_record: format!("VLD0:m{m}"),
            member_record_owner: format!("VLD0:o{m}"),
            joined,
        };
        s.members.insert(pk(m), row);
    }
    s
}

/// Apply `entry` by `author` at `lamport` into `s.successors`.
fn apply(s: &mut GovernanceState, author: u8, lamport: u64, entry: GovernanceEntry) {
    let GovernanceEntry::CommunitySuccessor(successor) = entry else {
        unreachable!("a successor entry")
    };
    let hash = OpHash([100 + u8::try_from(lamport).expect("small"); 32]);
    s.successors.insert(
        hash,
        SuccessorState {
            author: pk(author),
            successor,
            stamp: OpStamp { lamport, hash },
        },
    );
}

fn link(old: u8, new: &PseudonymKey, new_anchor: &str) -> CreatorLink {
    CreatorLink {
        old: pk(old),
        new: new.clone(),
        sig: key(old)
            .sign(&link_signing_bytes(OLD, new_anchor, new))
            .to_bytes()
            .to_vec(),
    }
}

#[test]
fn a_transfer_publishes_a_successor_adding_the_member_and_removing_the_signer() {
    let s = state();
    let set = transfer_set(&s, &pk(1), &pk(2));
    assert_eq!(set, [pk(2), pk(3)].into_iter().collect());
    let GovernanceEntry::CommunitySuccessor(offer) = offer_entry(&s, &pk(1), &pk(2)).unwrap()
    else {
        unreachable!()
    };
    assert_eq!(offer.shape(), Some(SuccessorShape::Offer));
    assert_eq!(offer.transfer_to, Some(pk(2)));
    assert_eq!(offer.creator_set_hash, Some(successor_set_hash(set.iter())));
}

#[test]
fn only_a_creator_offers_and_only_to_another_member() {
    let s = state();
    assert!(matches!(
        offer_entry(&s, &pk(2), &pk(4)),
        Err(GovernanceRuntimeError::PermissionDenied)
    ));
    assert!(offer_entry(&s, &pk(1), &pk(1)).is_err());
    assert!(offer_entry(&s, &pk(1), &pk(9)).is_err());
}

#[test]
fn a_transfer_completes_only_with_the_recipients_acceptance() {
    let mut s = state();
    let e = offer_entry(&s, &pk(1), &pk(2)).unwrap();
    apply(&mut s, 1, 5, e);
    let round = transfer_round(&s, &pk(2)).unwrap();
    assert_eq!(round.offerer, pk(1));
    assert_eq!(round.others, [pk(3)].into_iter().collect());
    assert!(
        transfer_round(&s, &pk(4)).is_err(),
        "no offer names member 4"
    );
}

#[test]
fn a_later_offer_supersedes_an_earlier_one() {
    let mut s = state();
    let e = offer_entry(&s, &pk(1), &pk(2)).unwrap();
    apply(&mut s, 1, 5, e);
    let e = offer_entry(&s, &pk(1), &pk(4)).unwrap();
    apply(&mut s, 1, 6, e);
    assert!(transfer_round(&s, &pk(2)).is_err());
    assert!(transfer_round(&s, &pk(4)).is_ok());
}

/// Lead decision (55): an offer stops standing once its acceptance is
/// applied, so a second accept finds no round (the Host reserves nothing)
/// and learns the successor already created.
#[test]
fn an_accepted_offer_stands_no_more() {
    let mut s = state();
    let e = offer_entry(&s, &pk(1), &pk(2)).unwrap();
    apply(&mut s, 1, 5, e);
    let round = transfer_round(&s, &pk(2)).expect("the offer stands");
    let offer = OpRef {
        op_hash: round.offer,
        author: pk(1),
        author_record: "VLD0:m1".into(),
        author_seq: 5,
    };
    let accepted = acceptance_entry(offer, round.set_hash, NEW, Vec::new(), "VLD0:inv");
    apply(&mut s, 2, 6, accepted);
    assert!(
        standing_offers(&s).is_empty(),
        "the accepted offer stands no more"
    );
    assert!(
        transfer_round(&s, &pk(2)).is_err(),
        "a second accept has no round"
    );
    assert_eq!(accepted_successor(&s, &pk(2)), Some(NEW));
    assert_eq!(accepted_successor(&s, &pk(4)), None);
}

#[test]
fn a_withdrawn_offer_cannot_be_accepted() {
    let mut s = state();
    let e = offer_entry(&s, &pk(1), &pk(2)).unwrap();
    apply(&mut s, 1, 5, e);
    apply(&mut s, 1, 6, withdrawal_entry());
    assert!(standing_offers(&s).is_empty());
    assert!(transfer_round(&s, &pk(2)).is_err());
}

#[test]
fn an_offer_over_a_changed_creator_set_must_be_made_again() {
    let mut s = state();
    let e = offer_entry(&s, &pk(1), &pk(2)).unwrap();
    apply(&mut s, 1, 5, e);
    s.creators.insert(pk(4));
    assert!(transfer_round(&s, &pk(2)).is_err());
}

#[test]
fn a_creator_link_binds_old_anchor_new_anchor_and_pseudonym() {
    let mut s = state();
    let e = offer_entry(&s, &pk(1), &pk(2)).unwrap();
    apply(&mut s, 1, 5, e);
    let round = transfer_round(&s, &pk(2)).unwrap();
    let (rn, cn) = (pk(20), pk(30));
    let good = [link(2, &rn, NEW), link(3, &cn, NEW)];
    assert!(check_links(OLD, NEW, &round, &pk(2), &good).is_ok());
    let elsewhere = [link(2, &rn, NEW), link(3, &cn, "VLD0:other")];
    assert!(check_links(OLD, NEW, &round, &pk(2), &elsewhere).is_err());
    let mut swapped = good.clone();
    swapped[1].new = pk(31);
    assert!(check_links(OLD, NEW, &round, &pk(2), &swapped).is_err());
}

#[test]
fn a_transfer_waits_for_every_co_creator_link() {
    let mut s = state();
    let e = offer_entry(&s, &pk(1), &pk(2)).unwrap();
    apply(&mut s, 1, 5, e);
    let round = transfer_round(&s, &pk(2)).unwrap();
    let only_mine = [link(2, &pk(20), NEW)];
    assert!(check_links(OLD, NEW, &round, &pk(2), &only_mine).is_err());
    let outsider = [link(2, &pk(20), NEW), link(4, &pk(40), NEW)];
    assert!(check_links(OLD, NEW, &round, &pk(2), &outsider).is_err());
}

fn op_ref(hash: OpHash, author: u8) -> OpRef {
    OpRef {
        op_hash: hash,
        author: pk(author),
        author_record: format!("VLD0:m{author}"),
        author_seq: 1,
    }
}

/// Lead decision (46), the lead's (b): a co-creator is told by a backend
/// event, once per session, that a link request asks it; once answered
/// the snapshot no longer lists it.
#[test]
fn a_link_request_asking_us_is_announced_once_until_answered() {
    let mut s = state();
    let e = offer_entry(&s, &pk(1), &pk(2)).unwrap();
    apply(&mut s, 1, 5, e);
    let request = GovernanceEntry::SuccessorLinkRequest(SuccessorLinkRequest {
        offer: op_ref(OpHash([105; 32]), 1),
        new_anchor: NEW.into(),
        asked: vec![pk(3)],
    });
    let req = OpHash([60; 32]);
    let at = |lamport, hash| OpStamp { lamport, hash };
    rekindle_governance::link_round::apply(&pk(2), &request, at(6, req), &mut s);

    let judgements = SuccessorJudgements::default();
    let first = new_link_requests("c", &s, &pk(3), &judgements);
    assert_eq!(first.len(), 1);
    assert!(
        matches!(&first[0], GovernanceRuntimeEvent::SuccessorLinkRequested {
        community_id, request, recipient, new_anchor
    } if community_id == "c" && *request == req && *recipient == pk(2) && new_anchor == NEW)
    );
    assert!(
        new_link_requests("c", &s, &pk(3), &judgements).is_empty(),
        "once per request per session"
    );
    let fresh = SuccessorJudgements::default();
    assert!(
        new_link_requests("c", &s, &pk(4), &fresh).is_empty(),
        "not asked"
    );

    let answer = GovernanceEntry::SuccessorLink(SuccessorLink {
        request: op_ref(req, 2),
        link: link(3, &pk(30), NEW),
        creator_sig: vec![0; 64],
    });
    rekindle_governance::link_round::apply(&pk(3), &answer, at(7, OpHash([61; 32])), &mut s);
    assert!(
        link_requests_asking("c", &s, &pk(3)).is_empty(),
        "answered: the snapshot drops it"
    );
}

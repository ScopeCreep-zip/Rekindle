//! `CommunitySuccessor` (plan E3.2 S7s.3e, ADR 0011, lead decisions
//! (24)–(26)): creator-only successors and offers, an acceptance only from
//! the offer's `transferTo` with the offer in its deps, its creator set
//! matching, and never of an offer its signer superseded.

use rekindle_types::governance::{
    AdmissionMode, CommunitySuccessor, CreatorLink, GovernanceEntry, SuccessorLink,
    SuccessorLinkRequest, SuccessorReason,
};

use crate::fixture::{cert, meta, op};
use crate::merge::merge_outcome;
use crate::merge::op_proptests::{community, pk};
use crate::merge::resume_tests::resume;
use crate::op::VerifiedOp;
use crate::successor::successor_set_hash;

fn successor() -> GovernanceEntry {
    GovernanceEntry::CommunitySuccessor(CommunitySuccessor {
        new_anchor: Some("VLD0:new".into()),
        reason: SuccessorReason::Recreate,
        transfer_to: None,
        creator_set_hash: None,
        accepts: None,
        links: Vec::new(),
        invites_record: Some("VLD0:inv".into()),
    })
}

fn hash_of(set: &[u8]) -> [u8; 32] {
    let keys: Vec<_> = set.iter().map(|b| pk(*b)).collect();
    successor_set_hash(&keys)
}

fn offer(to: u8, set: &[u8]) -> GovernanceEntry {
    GovernanceEntry::CommunitySuccessor(CommunitySuccessor {
        new_anchor: None,
        reason: SuccessorReason::Transfer,
        transfer_to: Some(pk(to)),
        creator_set_hash: Some(hash_of(set)),
        accepts: None,
        links: Vec::new(),
        invites_record: None,
    })
}

fn withdrawal() -> GovernanceEntry {
    GovernanceEntry::CommunitySuccessor(CommunitySuccessor {
        new_anchor: None,
        reason: SuccessorReason::Transfer,
        transfer_to: None,
        creator_set_hash: None,
        accepts: None,
        links: Vec::new(),
        invites_record: None,
    })
}

/// An acceptance of `offer` naming `set` as its hash and `links` as the
/// old pseudonyms it links.
fn acceptance(offer: &VerifiedOp, set: &[u8], links: &[u8]) -> GovernanceEntry {
    GovernanceEntry::CommunitySuccessor(CommunitySuccessor {
        new_anchor: Some("VLD0:new".into()),
        reason: SuccessorReason::Transfer,
        transfer_to: None,
        creator_set_hash: Some(hash_of(set)),
        accepts: Some(offer.op_ref("VLD0:fixture")),
        links: (links.iter())
            .map(|b| CreatorLink {
                old: pk(*b),
                new: pk(b + 100),
                sig: vec![0; 64],
            })
            .collect(),
        invites_record: Some("VLD0:inv".into()),
    })
}

/// `pk(4)`'s link request (its seq 1) for `o1` asking `pk(1)`, and
/// `pk(1)`'s link answering it (its seq 8, after `m`) with `new` as its
/// new pseudonym; `acceptance` carries `new == pk(101)` (lead decision
/// (46)).
fn link_round(
    o1: &VerifiedOp,
    d0: &VerifiedOp,
    m: &VerifiedOp,
    new: u8,
) -> (VerifiedOp, VerifiedOp) {
    let req = op(
        &pk(4),
        1,
        &[o1, d0],
        57,
        GovernanceEntry::SuccessorLinkRequest(SuccessorLinkRequest {
            offer: o1.op_ref("VLD0:fixture"),
            new_anchor: "VLD0:new".into(),
            asked: vec![pk(1)],
        }),
    );
    let link = op(
        &pk(1),
        8,
        &[m, &req],
        58,
        GovernanceEntry::SuccessorLink(SuccessorLink {
            request: req.op_ref("VLD0:fixture"),
            link: CreatorLink {
                old: pk(1),
                new: pk(new),
                sig: vec![0; 64],
            },
            creator_sig: vec![0; 64],
        }),
    );
    (req, link)
}

/// Whether `o` applied in the merge of `ops`, and is in `successors`.
fn kept(ops: &[VerifiedOp], o: &VerifiedOp) -> bool {
    let out = merge_outcome(&cert(&pk(1), AdmissionMode::Open), ops);
    let applied = out.applied.contains(&o.hash());
    assert_eq!(applied, out.state.successors.contains_key(&o.hash()));
    applied
}

/// `community()`'s ops by author: creator `pk(1)`, manager `pk(2)`
/// (`MANAGE_COMMUNITY`), members `pk(3)` and `pk(4)`.
fn at(ops: &[VerifiedOp], i: usize) -> &VerifiedOp {
    &ops[i]
}

#[test]
fn a_successor_signed_by_a_manager_is_refused() {
    let ops = community();
    let (a0, c5) = (at(&ops, 2), at(&ops, 8));
    let by_manager = op(&pk(2), 1, &[c5, a0], 50, successor());
    let offer_by_manager = op(&pk(2), 1, &[c5, a0], 51, offer(4, &[4]));
    let by_creator = op(&pk(1), 6, &[c5], 52, successor());
    for forged in [by_manager, offer_by_manager] {
        let mut all = ops.clone();
        all.push(forged.clone());
        assert!(!kept(&all, &forged), "a MANAGE_COMMUNITY holder is refused");
    }
    let mut all = ops.clone();
    all.push(by_creator.clone());
    assert!(kept(&all, &by_creator), "a creator's successor stands");
}

#[test]
fn a_successor_in_no_shape_is_refused() {
    let ops = community();
    let c5 = at(&ops, 8);
    let GovernanceEntry::CommunitySuccessor(mut bad) = offer(4, &[4]) else {
        unreachable!("an offer")
    };
    bad.new_anchor = Some("VLD0:new".into());
    let malformed = op(
        &pk(1),
        6,
        &[c5],
        53,
        GovernanceEntry::CommunitySuccessor(bad),
    );
    let mut all = ops.clone();
    all.push(malformed.clone());
    assert!(!kept(&all, &malformed), "the creator's malformed body too");
}

#[test]
fn an_acceptance_without_the_offer_in_its_deps_is_refused() {
    let mut ops = community();
    let (d0, c5) = (at(&ops, 4).clone(), at(&ops, 8).clone());
    let o1 = op(&pk(1), 6, &[&c5], 60, offer(4, &[4, 1]));
    let m = op(&pk(1), 7, &[&o1], 61, meta("after"));
    let (req, link) = link_round(&o1, &d0, &m, 101);
    let past_only = op(
        &pk(4),
        2,
        &[&m, &req, &link],
        62,
        acceptance(&o1, &[4, 1], &[1, 4]),
    );
    let in_deps = op(
        &pk(4),
        2,
        &[&m, &o1, &req, &link],
        63,
        acceptance(&o1, &[4, 1], &[1, 4]),
    );
    ops.extend([o1, m, req, link]);
    let mut all = ops.clone();
    all.push(past_only.clone());
    assert!(!kept(&all, &past_only), "the offer only in its causal past");
    all = ops;
    all.push(in_deps.clone());
    assert!(kept(&all, &in_deps), "the offer among its deps");
}

#[test]
fn an_acceptance_from_another_member_or_with_another_creator_set_is_refused() {
    let mut ops = community();
    let (b0, d0, c5) = (
        at(&ops, 3).clone(),
        at(&ops, 4).clone(),
        at(&ops, 8).clone(),
    );
    let o1 = op(&pk(1), 6, &[&c5], 70, offer(4, &[4, 1]));
    let m = op(&pk(1), 7, &[&o1], 69, meta("after"));
    let (req, link) = link_round(&o1, &d0, &m, 101);
    let cases = [
        // pk(3) is not the offer's transferTo.
        (
            op(
                &pk(3),
                1,
                &[&o1, &b0],
                71,
                acceptance(&o1, &[4, 1], &[1, 4]),
            ),
            false,
        ),
        // Links naming another set than the offer's hash.
        (
            op(
                &pk(4),
                1,
                &[&o1, &d0],
                72,
                acceptance(&o1, &[4, 1], &[4, 2]),
            ),
            false,
        ),
        // Another hash than the offer's, matching its own links.
        (
            op(&pk(4), 1, &[&o1, &d0], 73, acceptance(&o1, &[4], &[4])),
            false,
        ),
        // A set without transferTo, hashing to the offer's, citing its
        // link round.
        (
            op(
                &pk(4),
                2,
                &[&o1, &req, &link],
                74,
                acceptance(&o1, &[4, 1], &[4, 1]),
            ),
            true,
        ),
    ];
    ops.extend([o1, m, req, link]);
    for (acc, stands) in cases {
        let mut all = ops.clone();
        all.push(acc.clone());
        assert_eq!(kept(&all, &acc), stands, "{:?}", acc.body());
    }
}

#[test]
fn a_superseded_offer_cannot_be_accepted() {
    let ops = community();
    let (d0, c5) = (at(&ops, 4).clone(), at(&ops, 8).clone());
    let o1 = op(&pk(1), 6, &[&c5], 80, offer(4, &[4]));
    for later in [withdrawal(), offer(3, &[3])] {
        let w = op(&pk(1), 7, &[&o1], 81, later);
        let after = op(&pk(4), 1, &[&o1, &w, &d0], 82, acceptance(&o1, &[4], &[4]));
        let beside = op(&pk(4), 1, &[&o1, &d0], 83, acceptance(&o1, &[4], &[4]));
        let mut all = ops.clone();
        all.extend([o1.clone(), w.clone(), after.clone()]);
        assert!(
            !kept(&all, &after),
            "the signer's later offer or withdrawal in its past"
        );
        all = ops.clone();
        all.extend([o1.clone(), w, beside.clone()]);
        assert!(
            kept(&all, &beside),
            "concurrent with the withdrawal: not in its past"
        );
    }
}

#[test]
fn a_resumed_merge_judges_an_acceptance_of_a_cut_offer_alike() {
    let ops = community();
    let (d0, c5) = (at(&ops, 4).clone(), at(&ops, 8).clone());
    let o1 = op(&pk(1), 6, &[&c5], 90, offer(4, &[4]));
    let w = op(&pk(1), 7, &[&o1], 91, withdrawal());
    let after = op(&pk(4), 1, &[&o1, &w, &d0], 92, acceptance(&o1, &[4], &[4]));
    let open = op(&pk(4), 1, &[&o1, &d0], 93, acceptance(&o1, &[4], &[4]));
    let cert = cert(&pk(1), AdmissionMode::Open);

    let mut all = ops.clone();
    all.extend([o1.clone(), w.clone(), after.clone()]);
    let out = resume(&cert, &all, &[&w]).expect("on top");
    assert!(!out.state.successors.contains_key(&after.hash()));

    let mut all = ops;
    all.extend([o1.clone(), open.clone()]);
    let out = resume(&cert, &all, &[&o1]).expect("on top");
    assert!(out.state.successors.contains_key(&open.hash()));
}

/// Lead decision (46): a multi-creator acceptance stands only when its
/// deps cite its link request and, for every other creator it links, that
/// creator's link carrying exactly the acceptance's link.
#[test]
fn an_acceptance_not_citing_every_creators_link_is_refused() {
    let mut ops = community();
    let (d0, c5) = (at(&ops, 4).clone(), at(&ops, 8).clone());
    let o1 = op(&pk(1), 6, &[&c5], 100, offer(4, &[4, 1]));
    let m = op(&pk(1), 7, &[&o1], 101, meta("after"));
    let (req, link) = link_round(&o1, &d0, &m, 101);
    let (_, other) = link_round(&o1, &d0, &m, 102);
    let accept =
        |deps: &[&VerifiedOp], at: u64| op(&pk(4), 2, deps, at, acceptance(&o1, &[4, 1], &[4, 1]));
    let cases = [
        (accept(&[&o1, &req, &m], 102), false, "no link cited"),
        (accept(&[&o1, &link], 103), false, "the request not cited"),
        (accept(&[&o1, &req, &other], 104), false, "another link"),
        (accept(&[&o1, &req, &link], 105), true, "the whole round"),
    ];
    ops.extend([o1.clone(), m.clone(), req.clone(), link.clone()]);
    for (acc, stands, why) in cases {
        let mut all = ops.clone();
        if !stands && why == "another link" {
            // `other` replaces `link` as pk(1)'s seq 8.
            all.retain(|o| o.hash() != link.hash());
            all.push(other.clone());
        }
        all.push(acc.clone());
        assert_eq!(kept(&all, &acc), stands, "{why}");
    }
}

/// The resumed merge reads the link round from the cut's
/// `successor_links` and judges a rest acceptance as the full merge does.
#[test]
fn a_resumed_merge_judges_an_acceptance_citing_cut_links_alike() {
    let ops = community();
    let (d0, c5) = (at(&ops, 4).clone(), at(&ops, 8).clone());
    let o1 = op(&pk(1), 6, &[&c5], 110, offer(4, &[4, 1]));
    let m = op(&pk(1), 7, &[&o1], 111, meta("after"));
    let (req, link) = link_round(&o1, &d0, &m, 101);
    let cited = op(
        &pk(4),
        2,
        &[&o1, &req, &link],
        112,
        acceptance(&o1, &[4, 1], &[4, 1]),
    );
    let GovernanceEntry::CommunitySuccessor(mut wrong) = acceptance(&o1, &[4, 1], &[4, 1]) else {
        unreachable!("an acceptance")
    };
    wrong.links[1].new = pk(103);
    // pk(1)'s link in the acceptance is not the one it wrote.
    let mismatched = op(
        &pk(4),
        2,
        &[&o1, &req, &link],
        113,
        GovernanceEntry::CommunitySuccessor(wrong),
    );
    let cert = cert(&pk(1), AdmissionMode::Open);
    for (acc, stands) in [(cited, true), (mismatched, false)] {
        let mut all = ops.clone();
        all.extend([
            o1.clone(),
            m.clone(),
            req.clone(),
            link.clone(),
            acc.clone(),
        ]);
        let out = resume(&cert, &all, &[&link]).expect("on top");
        assert_eq!(out.state.successors.contains_key(&acc.hash()), stands);
        assert_eq!(kept(&all, &acc), stands, "the full merge alike");
    }
}

/// Lead decision (55): an offer accepted once is accepted no more; a
/// second acceptance of it is refused by the full merge, and the resumed
/// merge agrees whether the first acceptance is in the cut or the rest.
#[test]
fn a_second_acceptance_of_one_offer_is_refused() {
    let ops = community();
    let (d0, c5) = (at(&ops, 4).clone(), at(&ops, 8).clone());
    let o1 = op(&pk(1), 6, &[&c5], 120, offer(4, &[4]));
    let first = op(&pk(4), 1, &[&o1, &d0], 121, acceptance(&o1, &[4], &[4]));
    let second = op(&pk(4), 2, &[&o1, &first], 122, acceptance(&o1, &[4], &[4]));
    let cert = cert(&pk(1), AdmissionMode::Open);
    let mut all = ops;
    all.extend([o1.clone(), first.clone(), second.clone()]);
    assert!(kept(&all, &first), "the first acceptance stands");
    assert!(!kept(&all, &second), "a second acceptance is refused");
    for cut in [&o1, &first] {
        let out = resume(&cert, &all, &[cut]).expect("on top");
        assert!(out.state.successors.contains_key(&first.hash()));
        assert!(
            !out.state.successors.contains_key(&second.hash()),
            "the resumed merge refuses it too"
        );
    }
}

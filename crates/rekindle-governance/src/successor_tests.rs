//! The successor reader's rules over a verified far cert (plan E3.2
//! S7s.3e part 2, lead decision (24)).

use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
use rekindle_types::governance::{
    AdmissionMode, CommunitySuccessor, CreatorLink, GenesisCert, OpHash, SuccessorReason,
};
use rekindle_types::id::PseudonymKey;

use super::{link_signing_bytes, offered_creators, SuccessorRefusal, SuccessorState};
use crate::op::OpStamp;

const OLD: &str = "VLD0:old";
const NEW: &str = "VLD0:new";

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn pk(k: &SigningKey) -> PseudonymKey {
    PseudonymKey(k.verifying_key().to_bytes())
}

/// `old`'s link to `new` in the successor at `new_anchor` of `OLD`.
fn link(old: &SigningKey, new: &PseudonymKey, new_anchor: &str) -> CreatorLink {
    CreatorLink {
        old: pk(old),
        new: new.clone(),
        sig: old
            .sign(&link_signing_bytes(OLD, new_anchor, new))
            .to_bytes()
            .to_vec(),
    }
}

fn row(author: PseudonymKey, successor: CommunitySuccessor) -> SuccessorState {
    SuccessorState {
        author,
        successor,
        stamp: OpStamp {
            lamport: 1,
            hash: OpHash([1; 32]),
        },
    }
}

fn recreate() -> SuccessorState {
    row(
        pk(&key(1)),
        CommunitySuccessor {
            new_anchor: Some(NEW.into()),
            reason: SuccessorReason::Recreate,
            transfer_to: None,
            creator_set_hash: None,
            accepts: None,
            links: Vec::new(),
        },
    )
}

/// The new anchor's cert: `creator` plus `others`, naming `predecessor`.
fn cert(creator: &PseudonymKey, others: &[PseudonymKey], predecessor: Option<&str>) -> GenesisCert {
    let mut cert = crate::fixture::cert(creator, AdmissionMode::Open);
    cert.additional_creators = others.to_vec();
    cert.predecessor = predecessor.map(str::to_owned);
    cert
}

/// A transfer acceptance by recipient `r` (old key 2) with co-creator
/// old key 3, their new pseudonyms `rn` and `cn`, links signed over `NEW`.
fn acceptance(rn: &PseudonymKey, cn: &PseudonymKey) -> SuccessorState {
    row(
        pk(&key(2)),
        CommunitySuccessor {
            new_anchor: Some(NEW.into()),
            reason: SuccessorReason::Transfer,
            transfer_to: None,
            creator_set_hash: Some([9; 32]),
            accepts: Some(rekindle_types::governance::OpRef {
                op_hash: OpHash([7; 32]),
                author: pk(&key(1)),
                author_record: "VLD0:rec".into(),
                author_seq: 1,
            }),
            links: vec![link(&key(2), rn, NEW), link(&key(3), cn, NEW)],
        },
    )
}

#[test]
fn a_successor_without_the_predecessor_link_is_not_offered() {
    let creator = pk(&key(4));
    let none = offered_creators(OLD, &recreate(), &cert(&creator, &[], None));
    assert_eq!(none, Err(SuccessorRefusal::NoPredecessorLink));
    let other = offered_creators(OLD, &recreate(), &cert(&creator, &[], Some("VLD0:else")));
    assert_eq!(other, Err(SuccessorRefusal::NoPredecessorLink));
    let linked = offered_creators(OLD, &recreate(), &cert(&creator, &[], Some(OLD)));
    assert_eq!(linked, Ok(vec![creator]));
}

#[test]
fn an_offer_or_a_withdrawal_is_not_offered() {
    let mut offer = recreate();
    offer.successor.new_anchor = None;
    offer.successor.reason = SuccessorReason::Transfer;
    let refused = offered_creators(OLD, &offer, &cert(&pk(&key(4)), &[], Some(OLD)));
    assert_eq!(refused, Err(SuccessorRefusal::NotASuccessor));
}

#[test]
fn a_valid_acceptance_is_offered_with_the_cert_creator_set() {
    let (rn, cn) = (pk(&key(5)), pk(&key(6)));
    let offered = offered_creators(
        OLD,
        &acceptance(&rn, &cn),
        &cert(&rn, std::slice::from_ref(&cn), Some(OLD)),
    );
    assert_eq!(offered, Ok(vec![rn, cn]));
}

#[test]
fn a_creator_link_binds_old_anchor_new_anchor_and_pseudonym() {
    let (rn, cn) = (pk(&key(5)), pk(&key(6)));
    let good = cert(&rn, std::slice::from_ref(&cn), Some(OLD));
    // Signed over another new anchor.
    let mut other_anchor = acceptance(&rn, &cn);
    other_anchor.successor.links[1] = link(&key(3), &cn, "VLD0:other");
    assert_eq!(
        offered_creators(OLD, &other_anchor, &good),
        Err(SuccessorRefusal::BadLink)
    );
    // Signed by a key that is not the link's old pseudonym.
    let mut forged = acceptance(&rn, &cn);
    forged.successor.links[1].sig = link(&key(8), &cn, NEW).sig;
    assert_eq!(
        offered_creators(OLD, &forged, &good),
        Err(SuccessorRefusal::BadLink)
    );
    // Signed for another old community.
    let mut other_old = acceptance(&rn, &cn);
    other_old.successor.links[1].sig = key(3)
        .sign(&link_signing_bytes("VLD0:else", NEW, &cn))
        .to_bytes()
        .to_vec();
    assert_eq!(
        offered_creators(OLD, &other_old, &good),
        Err(SuccessorRefusal::BadLink)
    );
}

#[test]
fn an_acceptance_whose_links_are_not_the_cert_creator_set_is_not_offered() {
    let (rn, cn) = (pk(&key(5)), pk(&key(6)));
    let extra = cert(&rn, &[cn.clone(), pk(&key(7))], Some(OLD));
    assert_eq!(
        offered_creators(OLD, &acceptance(&rn, &cn), &extra),
        Err(SuccessorRefusal::CreatorSetMismatch)
    );
    let missing = cert(&rn, &[], Some(OLD));
    assert_eq!(
        offered_creators(OLD, &acceptance(&rn, &cn), &missing),
        Err(SuccessorRefusal::CreatorSetMismatch)
    );
}

#[test]
fn an_acceptance_whose_recipient_is_not_the_cert_creator_is_not_offered() {
    let (rn, cn) = (pk(&key(5)), pk(&key(6)));
    let swapped = cert(&cn, std::slice::from_ref(&rn), Some(OLD));
    assert_eq!(
        offered_creators(OLD, &acceptance(&rn, &cn), &swapped),
        Err(SuccessorRefusal::RecipientNotCreator)
    );
}

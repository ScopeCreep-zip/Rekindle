//! A member record is bound only with its owner's consent (ADR 0011
//! "Binding", review S7m part 4): a `MemberJoined` naming another member's
//! record, without `K_M`'s `ownerProof`, is refused before the merge, so a
//! front-runner can't take the victim's record and lock the victim out.

use super::*;
use crate::test_member::Member;
use rekindle_codec::governance_op::encode_op;
use rekindle_governance::fixture::meta;
use rekindle_governance::op::invite_proof_bytes;
use rekindle_governance::op::op_hash;
use rekindle_secrets::invite::{hash_invite_code, sign_invite_join};
use rekindle_secrets::member_record::sign_member_record_binding;
use rekindle_types::governance::{AdmissionMode, InviteProof};

const ANCHOR: &str = "VLD0:anchor";
const CODE: &str = "code";

impl Member {
    /// A join by `self` naming `victim`'s record. Its `ownerProof` is the
    /// one `victim`'s `K_M` signs: for `victim`'s own join, a binding to
    /// `victim`; replayed by anyone else, a binding to someone else.
    fn join_naming(&self, victim: &Self) -> GovernanceEntry {
        let record = victim.record();
        let code_hash = hash_invite_code(CODE);
        let invite = sign_invite_join(
            CODE,
            &invite_proof_bytes(ANCHOR, &self.author(), &record, &code_hash),
        )
        .to_vec();
        let owner_proof = sign_member_record_binding(
            &victim.owner.to_bytes(),
            ANCHOR,
            &victim.author().0,
            &record,
        );
        GovernanceEntry::MemberJoined {
            member_record: record,
            member_record_owner: victim.owner_text(),
            invite_proof: Some(InviteProof {
                code_hash,
                sig: invite,
            }),
            owner_proof: owner_proof.to_vec(),
        }
    }
}

fn cert(creator: &Member) -> GenesisCert {
    GenesisCert {
        name: "c".into(),
        policy: AdmissionMode::Open,
        initial_roles: Vec::new(),
        creator_pseudonym: creator.author(),
        creator_member_record: creator.record(),
        creator_member_record_owner: creator.owner_text(),
        created_at: 1,
        additional_creators: Vec::new(),
        predecessor: None,
    }
}

/// Sign `body` by `key` after exactly `past` (its deps are `past`'s
/// frontier), without verifying it as `build_op` does.
fn signed(
    cert: &GenesisCert,
    past: &[&HeldOp],
    key: &SigningKey,
    body: GovernanceEntry,
    issued_at: u64,
) -> HeldOp {
    let held_past: Vec<HeldOp> = past.iter().map(|h| (*h).clone()).collect();
    let set = verify_held(ANCHOR, &held_past);
    let state = merge::merge(cert, &set.ops);
    let author = PseudonymKey(key.verifying_key().to_bytes());
    let last = set
        .ops
        .iter()
        .filter(|o| *o.author() == author)
        .max_by_key(|o| o.op().seq);
    let (seq, prev) = last.map_or((0, None), |o| (o.op().seq + 1, Some(o.hash())));
    let deps = frontier(&set.ops, prev)
        .into_iter()
        .map(|o| o.op_ref(&author_record(cert, &state, o.author()).unwrap()))
        .collect();
    let mut op = GovernanceOp {
        author,
        seq,
        prev,
        deps,
        issued_at,
        body,
        sig: [0; 64],
    };
    let unsigned = op_unsigned_bytes(&op);
    op.sig = key.sign(&op_signing_bytes(ANCHOR, &unsigned)).to_bytes();
    HeldOp {
        hash: op_hash(&unsigned, &op.sig),
        author: op.author.clone(),
        author_record: String::new(),
        seq,
        lamport: 0,
        issued_at,
        encoded: encode_op(&op).unwrap(),
        fate: OpFate::PendingDeps,
    }
}

fn invite() -> GovernanceEntry {
    GovernanceEntry::InviteCreated {
        invite_id: [1; 16],
        code_hash: hash_invite_code(CODE),
        max_uses: 8,
        expires_at: None,
        secrets_record_key: "VLD0:s".into(),
    }
}

/// The victim holds its own record after the merge of `held`, and `bad`
/// never verified.
fn assert_victim_keeps_its_record(
    cert: &GenesisCert,
    held: &[HeldOp],
    victim: &Member,
    bad: &HeldOp,
) {
    let set = verify_held(ANCHOR, held);
    let state = merge::merge(cert, &set.ops);
    let bound = state
        .members
        .get(&victim.author())
        .map(|m| m.member_record.clone());
    assert_eq!(
        bound,
        Some(victim.record()),
        "the victim is locked out of its record"
    );
    assert!(
        set.quarantined.contains(&bad.hash),
        "the unproven binding verified"
    );
}

#[test]
fn an_invite_holder_front_running_a_join_cannot_take_the_victims_record() {
    let (creator, victim, attacker) = (Member::new(1), Member::new(2), Member::new(3));
    let cert = cert(&creator);
    let inv = signed(&cert, &[], &creator.pseudonym, invite(), 10);
    let later = signed(&cert, &[&inv], &creator.pseudonym, meta("later"), 11);
    // The victim's join follows `later` (lamport 3); the attacker's names
    // the victim's record and owner, replays its proof, and cites only the
    // invite (lamport 2), so it comes first in stamp order.
    let theirs = signed(
        &cert,
        &[&inv, &later],
        &victim.pseudonym,
        victim.join_naming(&victim),
        12,
    );
    let front = signed(
        &cert,
        &[&inv],
        &attacker.pseudonym,
        attacker.join_naming(&victim),
        12,
    );
    assert_victim_keeps_its_record(&cert, &[inv, later, front.clone(), theirs], &victim, &front);
}

#[test]
fn a_member_re_joining_on_the_victims_record_cannot_take_it() {
    let (creator, victim, member) = (Member::new(1), Member::new(2), Member::new(4));
    let cert = cert(&creator);
    let inv = signed(&cert, &[], &creator.pseudonym, invite(), 10);
    let own = signed(
        &cert,
        &[&inv],
        &member.pseudonym,
        member.join_naming(&member),
        11,
    );
    let m2 = signed(&cert, &[&inv], &creator.pseudonym, meta("b"), 12);
    let m3 = signed(&cert, &[&inv, &m2], &creator.pseudonym, meta("c"), 13);
    // The member's second join (lamport 3) names the victim's record; the
    // victim's own join follows `m3` (lamport 4).
    let rejoin = signed(
        &cert,
        &[&inv, &own],
        &member.pseudonym,
        member.join_naming(&victim),
        14,
    );
    let theirs = signed(
        &cert,
        &[&inv, &m2, &m3],
        &victim.pseudonym,
        victim.join_naming(&victim),
        14,
    );
    let set = verify_held(ANCHOR, &[inv.clone(), own.clone()]);
    assert!(merge::merge(&cert, &set.ops)
        .members
        .contains_key(&member.author()));
    assert_victim_keeps_its_record(
        &cert,
        &[inv, own, m2, m3, rejoin.clone(), theirs],
        &victim,
        &rejoin,
    );
}

/// Plan E3.2 S10 (found, security): a party that read the op log but never
/// held the invite code (a kicked member, an I16 outsider) knows the
/// invite's public `code_hash` and its own keys, and nothing else. Its
/// `MemberJoined` citing the live invite, with the best proof it can make
/// (its own pseudonym's signature over the proof bytes), must not verify,
/// and it must not be a member after the merge.
#[test]
fn an_op_log_reader_without_the_invite_code_cannot_join() {
    let (creator, outsider) = (Member::new(1), Member::new(5));
    let cert = cert(&creator);
    let inv = signed(&cert, &[], &creator.pseudonym, invite(), 10);
    let record = outsider.record();
    // All it knows of the invite is the public `code_hash` in the log.
    let code_hash = hash_invite_code(CODE);
    let proof = invite_proof_bytes(ANCHOR, &outsider.author(), &record, &code_hash);
    let sig = outsider.pseudonym.sign(&proof).to_bytes().to_vec();
    let owner_proof = sign_member_record_binding(
        &outsider.owner.to_bytes(),
        ANCHOR,
        &outsider.author().0,
        &record,
    );
    let forged = GovernanceEntry::MemberJoined {
        member_record: record,
        member_record_owner: outsider.owner_text(),
        invite_proof: Some(InviteProof { code_hash, sig }),
        owner_proof: owner_proof.to_vec(),
    };
    let join = signed(&cert, &[&inv], &outsider.pseudonym, forged, 11);
    let set = verify_held(ANCHOR, &[inv, join.clone()]);
    let state = merge::merge(&cert, &set.ops);
    assert!(
        !state.members.contains_key(&outsider.author()),
        "a join without the invite code was admitted"
    );
    assert!(
        set.quarantined.contains(&join.hash),
        "the forged proof verified"
    );
}

/// A real joiner's proof names its joiner: copied onto another pseudonym's
/// `MemberJoined` (same invite, the copier's own record), it does not
/// verify, so a reader of the op log cannot replay a proof it saw.
#[test]
fn a_seen_invite_proof_cannot_be_replayed_for_another_joiner() {
    let (creator, joiner, copier) = (Member::new(1), Member::new(2), Member::new(6));
    let cert = cert(&creator);
    let inv = signed(&cert, &[], &creator.pseudonym, invite(), 10);
    let theirs = signed(
        &cert,
        &[&inv],
        &joiner.pseudonym,
        joiner.join_naming(&joiner),
        11,
    );
    let GovernanceEntry::MemberJoined { invite_proof, .. } = joiner.join_naming(&joiner) else {
        unreachable!()
    };
    let GovernanceEntry::MemberJoined {
        member_record,
        member_record_owner,
        owner_proof,
        ..
    } = copier.join_naming(&copier)
    else {
        unreachable!()
    };
    let replayed = GovernanceEntry::MemberJoined {
        member_record,
        member_record_owner,
        invite_proof,
        owner_proof,
    };
    let copy = signed(&cert, &[&inv, &theirs], &copier.pseudonym, replayed, 12);
    let set = verify_held(ANCHOR, &[inv, theirs, copy.clone()]);
    let state = merge::merge(&cert, &set.ops);
    assert!(state.members.contains_key(&joiner.author()));
    assert!(
        !state.members.contains_key(&copier.author()),
        "a replayed proof was admitted"
    );
    assert!(
        set.quarantined.contains(&copy.hash),
        "the replayed proof verified"
    );
}

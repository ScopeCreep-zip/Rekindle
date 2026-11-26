//! A device's ops land on its own op record (ADR 0022 rule 3, plan E3.2
//! K5i): the record its key's endorsement binds in the merged device set,
//! `M(p)` for the first device, `M(p,d)` for any other; an unendorsed key's
//! op is refused, never written to `M(p)`.

use std::sync::Arc;

use parking_lot::Mutex;
use rekindle_governance::device_writer::{endorse, DeviceRecord};
use rekindle_governance::merge;
use rekindle_governance::op::VerifiedOp;
use rekindle_governance::state::GovernanceState;
use rekindle_secrets::derive;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::{DeviceEndorsement, GenesisCert, GovernanceEntry, GovernanceOp};
use rekindle_types::id::PseudonymKey;
use zeroize::Zeroizing;

use super::write_entry;
use crate::deps::{CommunityMembership, MockGovernanceRuntimeDeps, OpOutcome};
use crate::device_signer::{first_endorsement, op_record_of, own_op_record, OpSigner};
use crate::error::GovernanceRuntimeError;
use crate::op_log::{build_op, Deps, HeldOp, VerifiedSet};
use crate::unlock_barrier::{BarrierPhase, UnlockBarrier};

const HELD_IN: &str = crate::test_world::ANCHOR;
const SECRET: [u8; 32] = [9; 32];
const MEMBER_RECORD: &str = "VLD0:mp";
const DEVICE_RECORD: &str = "VLD0:mpd";
const FIRST_DEVICE: [u8; 32] = [8; 32];
const SECOND_DEVICE: [u8; 32] = [7; 32];

/// The creator's state: its first device `FIRST_DEVICE` bound to `M(p)` by the
/// cert, and its second device `SECOND_DEVICE` endorsed for `M(p,d)`.
fn two_device_state() -> (GenesisCert, GovernanceState) {
    let p = derive::derive_community_pseudonym(&SECRET, HELD_IN);
    let author = PseudonymKey(p.verifying_key().to_bytes());
    let first = SigningKey::from_bytes(&FIRST_DEVICE);
    let cert = GenesisCert {
        name: "c".into(),
        policy: rekindle_types::governance::AdmissionMode::Open,
        initial_roles: Vec::new(),
        creator_pseudonym: author.clone(),
        creator_member_record: MEMBER_RECORD.into(),
        creator_member_record_owner: "o".into(),
        created_at: 1,
        additional_creators: Vec::new(),
        predecessor: None,
        creator_device: Some(first_endorsement(&p, HELD_IN, &first, MEMBER_RECORD, "o")),
    };
    // A held genesis op seeds the cert's device in a merge; with none
    // held, the fixture seeds it as that merge would.
    let mut state = merge::merge(&cert, &[]);
    state.devices.seed_genesis(HELD_IN, &cert);
    let second = SigningKey::from_bytes(&SECOND_DEVICE)
        .verifying_key()
        .to_bytes();
    let record = DeviceRecord {
        record: DEVICE_RECORD.into(),
        owner: "opd".into(),
    };
    let held = state.devices.member(&author);
    let e = endorse(&p, HELD_IN, MEMBER_RECORD, held, second, record, [0; 32]).unwrap();
    let carrier = GovernanceOp {
        author,
        seq: 0,
        prev: None,
        deps: Vec::new(),
        issued_at: 0,
        body: GovernanceEntry::DeviceKey(e),
        sig: [0; 64],
        device_key: second,
    };
    state.devices.absorb(HELD_IN, &carrier);
    (cert, state)
}

/// Deps for a write by the device holding `device` over `state`; the held
/// rows it stores land in the returned list.
fn deps_for(
    cert: GenesisCert,
    state: GovernanceState,
    device: [u8; 32],
) -> (MockGovernanceRuntimeDeps, Arc<Mutex<Vec<HeldOp>>>) {
    let barrier = Arc::new(UnlockBarrier::default());
    barrier.complete(Some(BarrierPhase::Passed));
    deps_behind(cert, state, device, barrier)
}

/// [`deps_for`] behind `barrier`; a held write is answered and dropped.
fn deps_behind(
    cert: GenesisCert,
    state: GovernanceState,
    device: [u8; 32],
    barrier: Arc<UnlockBarrier>,
) -> (MockGovernanceRuntimeDeps, Arc<Mutex<Vec<HeldOp>>>) {
    let mut deps = MockGovernanceRuntimeDeps::new();
    let mut ops = crate::deps::MockGovernanceOpStore::new();
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    deps.expect_rebuild_lock()
        .returning(move |_| Arc::clone(&lock));
    deps.expect_governance_state()
        .returning(move |_| Some(state.clone()));
    let hex = cert.creator_pseudonym.to_hex();
    // The write path judges successors in the state it installs (SS4h).
    deps.expect_successor_judgements()
        .returning(|_| Arc::default());
    deps.expect_community_membership().returning(move |_| {
        Some(CommunityMembership {
            anchor: Some(HELD_IN.into()),
            member_registry_key: None,
            my_pseudonym_hex: Some(hex.clone()),
            my_subkey_index: None,
            my_segment_index: None,
            slot_keypair: None,
            slot_seed_hex: None,
            channel_ids: Vec::new(),
            mek_generation: 0,
        })
    });
    deps.expect_genesis_cert()
        .returning(move |_| Some(cert.clone()));
    ops.expect_governance_ops().returning(|_| Ok(Vec::new()));
    deps.expect_op_verify_cache()
        .returning(|_| Arc::new(crate::verify_cache::OpVerifyCache::default()));
    deps.expect_identity_secret().returning(|| Some(SECRET));
    deps.expect_device_op_key()
        .returning(move |_| Ok(Some(Zeroizing::new(device))));
    deps.expect_clock()
        .returning(|| Arc::new(rekindle_utils::SystemClock));
    let stored = Arc::new(Mutex::new(Vec::new()));
    let into = Arc::clone(&stored);
    ops.expect_store_signed_governance_op()
        .returning(move |_, op| {
            into.lock().push(op.clone());
            Ok(())
        });
    let ops: std::sync::Arc<dyn crate::deps::GovernanceOpStore> = std::sync::Arc::new(ops);
    deps.expect_op_store()
        .returning(move || std::sync::Arc::clone(&ops));
    deps.expect_gossip_governance_op().return_const(());
    deps.expect_set_governance_state().return_const(());
    deps.expect_emit_event().return_const(());
    deps.expect_unlock_barrier()
        .returning(move |_| Arc::clone(&barrier));
    deps.expect_emit_catch_up().return_const(());
    deps.expect_pending_target_name().returning(|_, _| None);
    deps.expect_handle()
        .returning(MockGovernanceRuntimeDeps::new);
    deps.expect_spawn_held_write()
        .returning(|_, write| drop(write));
    deps.expect_publish_governance_op()
        .returning(|_, _, _| Ok(crate::deps::OpPublication::settled(Ok(OpOutcome::Landed))));
    (deps, stored)
}

async fn write_as(device: [u8; 32]) -> (Result<OpOutcome, GovernanceRuntimeError>, Vec<HeldOp>) {
    let (cert, state) = two_device_state();
    let (deps, stored) = deps_for(cert, state, device);
    let entry = rekindle_governance::fixture::meta("renamed");
    let written = write_entry(&deps, HELD_IN, entry).await;
    let held = stored.lock().clone();
    (written, held)
}

/// A second device writes its op to `M(p,d)`, the record its endorsement
/// binds, and the outbox's resolver publishes on the same record.
#[tokio::test]
async fn a_second_device_writes_and_publishes_on_its_own_record() {
    let (written, held) = write_as(SECOND_DEVICE).await;
    assert!(matches!(written, Ok(OpOutcome::Landed)), "{written:?}");
    let [row] = held.as_slice() else {
        panic!("one op stored");
    };
    assert_eq!(
        row.author_record, DEVICE_RECORD,
        "the second device's op lands on M(p,d)"
    );

    let (cert, state) = two_device_state();
    let (deps, _) = deps_for(cert, state.clone(), SECOND_DEVICE);
    assert_eq!(
        own_op_record(&deps, HELD_IN, &state).unwrap(),
        DEVICE_RECORD
    );
}

/// The first device's endorsement names `M(p)`: unchanged.
#[tokio::test]
async fn the_first_device_still_writes_on_the_member_record() {
    let (written, held) = write_as(FIRST_DEVICE).await;
    assert!(matches!(written, Ok(OpOutcome::Landed)), "{written:?}");
    let [row] = held.as_slice() else {
        panic!("one op stored");
    };
    assert_eq!(row.author_record, MEMBER_RECORD);
}

/// A key the device set never endorsed writes nothing: refused by name,
/// never stored on `M(p)`.
#[tokio::test]
async fn an_unendorsed_device_key_is_refused_not_written_to_the_member_record() {
    let (written, held) = write_as([6; 32]).await;
    assert!(
        matches!(written, Err(GovernanceRuntimeError::DeviceUnendorsed(ref c)) if c == HELD_IN),
        "{written:?}"
    );
    assert!(held.is_empty(), "nothing stored for an unendorsed key");
}

/// A write by `device` while the unlock barrier catches up (plan E3.2
/// S7d-keys.8), when an endorsed device's write is held.
async fn write_catching_up(
    device: [u8; 32],
) -> (Result<OpOutcome, GovernanceRuntimeError>, Vec<HeldOp>) {
    let (cert, state) = two_device_state();
    let barrier = Arc::new(UnlockBarrier::default());
    assert!(matches!(barrier.phase(), BarrierPhase::CatchingUp));
    let (deps, stored) = deps_behind(cert, state, device, barrier);
    let entry = rekindle_governance::fixture::meta("renamed");
    let written = write_entry(&deps, HELD_IN, entry).await;
    let held = stored.lock().clone();
    (written, held)
}

/// ADR 0022 rule 3 at the call: an unendorsed key's write is refused at
/// once while the barrier catches up, never answered "held" and dropped
/// when its turn comes; an endorsed device's write is still held.
#[tokio::test]
async fn an_unendorsed_device_write_is_refused_at_the_call_never_held() {
    let (written, held) = write_catching_up([6; 32]).await;
    assert!(
        matches!(written, Err(GovernanceRuntimeError::DeviceUnendorsed(ref c)) if c == HELD_IN),
        "{written:?}"
    );
    assert!(held.is_empty(), "nothing stored for an unendorsed key");
    let (endorsed, _) = write_catching_up(SECOND_DEVICE).await;
    assert!(
        matches!(endorsed, Ok(OpOutcome::Held { .. })),
        "{endorsed:?}"
    );
}

/// An op signed by `device` over `set`, citing its frontier.
fn signed_by(
    cert: &GenesisCert,
    state: &GovernanceState,
    set: &VerifiedSet,
    device: [u8; 32],
) -> VerifiedOp {
    let key = SigningKey::from_bytes(&device);
    let signer = OpSigner {
        author: cert.creator_pseudonym.clone(),
        device: &key,
    };
    let body = rekindle_governance::fixture::meta("x");
    build_op(HELD_IN, cert, state, set, signer, body, Deps::Frontier, 1)
        .unwrap()
        .0
}

/// A reference to an op names the record that op lives on (ADR 0022 rule
/// 3): a second device's op its `M(p,d)`, so a reader fetching the dep
/// finds it; a first device's op `M(p)`.
#[test]
fn a_dependency_names_the_record_its_op_lives_on() {
    let (cert, state) = two_device_state();
    let cases = [
        (SECOND_DEVICE, FIRST_DEVICE, DEVICE_RECORD),
        (FIRST_DEVICE, SECOND_DEVICE, MEMBER_RECORD),
    ];
    for (author, citer, want) in cases {
        let cited = signed_by(&cert, &state, &VerifiedSet::default(), author);
        let set = VerifiedSet {
            ops: vec![cited.clone()],
            ..VerifiedSet::default()
        };
        let citing = signed_by(&cert, &state, &set, citer);
        let [dep] = citing.op().deps.as_slice() else {
            panic!("one dep");
        };
        assert_eq!(dep.op_hash, cited.hash());
        assert_eq!(dep.author_record, want, "the dep on {author:?}'s op");
    }
}

fn joined(member_record: &str, first_device: Option<DeviceEndorsement>) -> GovernanceOp {
    let p = derive::derive_community_pseudonym(&SECRET, HELD_IN);
    GovernanceOp {
        author: PseudonymKey(p.verifying_key().to_bytes()),
        seq: 0,
        prev: None,
        deps: Vec::new(),
        issued_at: 0,
        body: GovernanceEntry::MemberJoined {
            member_record: member_record.into(),
            member_record_owner: "o2".into(),
            invite_proof: None,
            owner_proof: vec![0; 64],
            first_device,
        },
        sig: [0; 64],
        device_key: SigningKey::from_bytes(&FIRST_DEVICE)
            .verifying_key()
            .to_bytes(),
    }
}

/// Our `MemberJoined` routes under the endorsement it carries (a re-join's
/// new `M(p)`, which the state does not hold yet), only when that is `p`'s
/// of our held key for the record it names (ADR 0022 rules 3, 10).
#[test]
fn a_member_joined_routes_under_the_endorsement_it_carries() {
    let (cert, state) = two_device_state();
    let (deps, _) = deps_for(cert, state.clone(), FIRST_DEVICE);
    let p = derive::derive_community_pseudonym(&SECRET, HELD_IN);
    let ours = SigningKey::from_bytes(&FIRST_DEVICE);
    let rejoin = "VLD0:mp2";
    let e = first_endorsement(&p, HELD_IN, &ours, rejoin, "o2");
    let routed = op_record_of(&deps, HELD_IN, &state, &joined(rejoin, Some(e)));
    assert_eq!(routed.unwrap(), rejoin);

    let forger = SigningKey::from_bytes(&[3; 32]);
    let forged = first_endorsement(&forger, HELD_IN, &ours, rejoin, "o2");
    let theirs = SigningKey::from_bytes(&[5; 32]);
    let other_key = first_endorsement(&p, HELD_IN, &theirs, rejoin, "o2");
    for refused in [Some(forged), Some(other_key), None] {
        let routed = op_record_of(&deps, HELD_IN, &state, &joined(rejoin, refused));
        assert!(
            matches!(routed, Err(GovernanceRuntimeError::DeviceUnendorsed(_))),
            "{routed:?}"
        );
    }
}

/// Our `DeviceKey` of our own key lands on the `M(p,d)` it binds, routed
/// under the endorsement it carries before the state holds it (the
/// enrolment's own write); another key's resolves from the state, as
/// every later op.
#[test]
fn a_device_key_op_routes_to_the_record_it_binds() {
    let (cert, state) = two_device_state();
    let second = SigningKey::from_bytes(&SECOND_DEVICE)
        .verifying_key()
        .to_bytes();
    let md = state.devices.member(&cert.creator_pseudonym).unwrap();
    let e = (md.endorsements.values())
        .find(|e| e.device_key == second)
        .unwrap()
        .clone();
    let mut carrier = joined(MEMBER_RECORD, None);
    carrier.body = GovernanceEntry::DeviceKey(e);
    carrier.device_key = second;
    // The state before the enrolment is applied: the cert's device alone.
    let mut before = merge::merge(&cert, &[]);
    before.devices.seed_genesis(HELD_IN, &cert);
    let (deps, _) = deps_for(cert.clone(), before.clone(), SECOND_DEVICE);
    let routed = op_record_of(&deps, HELD_IN, &before, &carrier).unwrap();
    assert_eq!(routed, DEVICE_RECORD);
    let (deps, _) = deps_for(cert, before.clone(), FIRST_DEVICE);
    let routed = op_record_of(&deps, HELD_IN, &before, &carrier).unwrap();
    assert_eq!(routed, MEMBER_RECORD);
}

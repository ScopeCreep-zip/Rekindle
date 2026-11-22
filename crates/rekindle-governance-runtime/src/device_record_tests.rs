//! A device's ops land on its own op record (ADR 0022 rule 3, plan E3.2
//! K5i): the record its key's endorsement binds in the merged device set,
//! `M(p)` for the first device, `M(p,d)` for any other; an unendorsed key's
//! op is refused, never written to `M(p)`.

use std::sync::Arc;

use parking_lot::Mutex;
use rekindle_governance::device_writer::{endorse, DeviceRecord};
use rekindle_governance::merge;
use rekindle_governance::state::GovernanceState;
use rekindle_secrets::derive;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::{GenesisCert, GovernanceEntry, GovernanceOp};
use rekindle_types::id::PseudonymKey;
use zeroize::Zeroizing;

use super::write_entry;
use crate::deps::{CommunityMembership, MockGovernanceRuntimeDeps, OpOutcome};
use crate::device_signer::{first_endorsement, own_op_record};
use crate::error::GovernanceRuntimeError;
use crate::op_log::HeldOp;

const COMMUNITY: &str = crate::test_world::ANCHOR;
const SECRET: [u8; 32] = [9; 32];
const M_P: &str = "VLD0:mp";
const M_PD: &str = "VLD0:mpd";
const FIRST: [u8; 32] = [8; 32];
const SECOND: [u8; 32] = [7; 32];

/// The creator's state: its first device `FIRST` bound to `M(p)` by the
/// cert, and its second device `SECOND` endorsed for `M(p,d)`.
fn two_device_state() -> (GenesisCert, GovernanceState) {
    let p = derive::derive_community_pseudonym(&SECRET, COMMUNITY);
    let author = PseudonymKey(p.verifying_key().to_bytes());
    let first = SigningKey::from_bytes(&FIRST);
    let cert = GenesisCert {
        name: "c".into(),
        policy: rekindle_types::governance::AdmissionMode::Open,
        initial_roles: Vec::new(),
        creator_pseudonym: author.clone(),
        creator_member_record: M_P.into(),
        creator_member_record_owner: "o".into(),
        created_at: 1,
        additional_creators: Vec::new(),
        predecessor: None,
        creator_device: Some(first_endorsement(&p, COMMUNITY, &first, M_P, "o").unwrap()),
    };
    // A held genesis op seeds the cert's device in a merge; with none
    // held, the fixture seeds it as that merge would.
    let mut state = merge::merge(&cert, &[]);
    state.devices.seed_genesis(COMMUNITY, &cert);
    let second = SigningKey::from_bytes(&SECOND).verifying_key().to_bytes();
    let record = DeviceRecord {
        record: M_PD.into(),
        owner: "opd".into(),
    };
    let held = state.devices.member(&author);
    let e = endorse(&p, COMMUNITY, M_P, held, second, record, [0; 32]).unwrap();
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
    state.devices.absorb(COMMUNITY, &carrier);
    (cert, state)
}

/// Deps for a write by the device holding `device` over `state`; the held
/// rows it stores land in the returned list.
fn deps_for(
    cert: GenesisCert,
    state: GovernanceState,
    device: [u8; 32],
) -> (MockGovernanceRuntimeDeps, Arc<Mutex<Vec<HeldOp>>>) {
    let mut deps = MockGovernanceRuntimeDeps::new();
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    deps.expect_rebuild_lock()
        .returning(move |_| Arc::clone(&lock));
    deps.expect_governance_state()
        .returning(move |_| Some(state.clone()));
    let hex = cert.creator_pseudonym.to_hex();
    deps.expect_community_membership().returning(move |_| {
        Some(CommunityMembership {
            anchor: Some(COMMUNITY.into()),
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
    deps.expect_governance_ops().returning(|_| Ok(Vec::new()));
    deps.expect_op_verify_cache()
        .returning(|_| Arc::new(crate::verify_cache::OpVerifyCache::default()));
    deps.expect_identity_secret().returning(|| Some(SECRET));
    deps.expect_device_op_key()
        .returning(move |_| Ok(Some(Zeroizing::new(device))));
    deps.expect_clock()
        .returning(|| Arc::new(rekindle_utils::SystemClock));
    let stored = Arc::new(Mutex::new(Vec::new()));
    let into = Arc::clone(&stored);
    deps.expect_store_signed_governance_op()
        .returning(move |_, op| {
            into.lock().push(op.clone());
            Ok(())
        });
    deps.expect_gossip_governance_op().return_const(());
    deps.expect_set_governance_state().return_const(());
    deps.expect_emit_event().return_const(());
    deps.expect_publish_governance_op()
        .returning(|_, _, _| Ok(OpOutcome::Landed));
    (deps, stored)
}

async fn write_as(device: [u8; 32]) -> (Result<OpOutcome, GovernanceRuntimeError>, Vec<HeldOp>) {
    let (cert, state) = two_device_state();
    let (deps, stored) = deps_for(cert, state, device);
    let entry = rekindle_governance::fixture::meta("renamed");
    let written = write_entry(&deps, COMMUNITY, entry).await;
    let held = stored.lock().clone();
    (written, held)
}

/// A second device writes its op to `M(p,d)`, the record its endorsement
/// binds, and the outbox's resolver publishes on the same record.
#[tokio::test]
async fn a_second_device_writes_and_publishes_on_its_own_record() {
    let (written, held) = write_as(SECOND).await;
    assert!(matches!(written, Ok(OpOutcome::Landed)), "{written:?}");
    let [row] = held.as_slice() else {
        panic!("one op stored");
    };
    assert_eq!(
        row.author_record, M_PD,
        "the second device's op lands on M(p,d)"
    );

    let (cert, state) = two_device_state();
    let (deps, _) = deps_for(cert, state.clone(), SECOND);
    assert_eq!(own_op_record(&deps, COMMUNITY, &state).unwrap(), M_PD);
}

/// The first device's endorsement names `M(p)`: unchanged.
#[tokio::test]
async fn the_first_device_still_writes_on_the_member_record() {
    let (written, held) = write_as(FIRST).await;
    assert!(matches!(written, Ok(OpOutcome::Landed)), "{written:?}");
    let [row] = held.as_slice() else {
        panic!("one op stored");
    };
    assert_eq!(row.author_record, M_P);
}

/// A key the device set never endorsed writes nothing: refused by name,
/// never stored on `M(p)`.
#[tokio::test]
async fn an_unendorsed_device_key_is_refused_not_written_to_the_member_record() {
    let (written, held) = write_as([6; 32]).await;
    assert!(
        matches!(written, Err(GovernanceRuntimeError::DeviceUnendorsed(ref c)) if c == COMMUNITY),
        "{written:?}"
    );
    assert!(held.is_empty(), "nothing stored for an unendorsed key");
}

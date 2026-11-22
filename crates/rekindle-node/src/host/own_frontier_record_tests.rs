//! Our frontier republish (`republish_own_frontier`, `M`/3) is written on
//! our device's op record with its owner (ADR 0022 rule 3, plan E3.2 K5i):
//! `M(p,d)` with `K_M(p,d)` for a second device, never the person's `M(p)`
//! with `K_M(p)`; the first device's stays `M(p)` with `K_M(p)`.

use std::sync::Arc;

use rekindle_governance::device_writer::{endorse, DeviceRecord};
use rekindle_governance::merge;
use rekindle_governance::state::GovernanceState;
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
use rekindle_governance_runtime::device_signer::first_endorsement;
use rekindle_secrets::derive;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::{GenesisCert, GovernanceEntry, GovernanceOp};
use rekindle_types::id::PseudonymKey;

use super::{create_ann, offline_host, seed_registry_community};
use crate::host::adapters::governance::HostGovernance;

const COMMUNITY: &str = "c1";
const PERSON_SECRET: [u8; 32] = [9; 32];
const PERSON_RECORD: &str = "VLD0:mp";
const PERSON_OWNER: &str = "K_M(p)";
const PHONE_RECORD: &str = "VLD0:mpd";
const PHONE_OWNER: &str = "K_M(p,d)";
const LAPTOP_KEY: [u8; 32] = [8; 32];
const PHONE_KEY: [u8; 32] = [7; 32];

/// The person's cert (its first device `LAPTOP_KEY` bound to `M(p)`) and a
/// state that also endorses its second device `PHONE_KEY` for `M(p,d)`.
fn two_device_state() -> (PseudonymKey, GenesisCert, GovernanceState) {
    let p = derive::derive_community_pseudonym(&PERSON_SECRET, COMMUNITY);
    let author = PseudonymKey(p.verifying_key().to_bytes());
    let laptop = SigningKey::from_bytes(&LAPTOP_KEY);
    let creator_device =
        first_endorsement(&p, COMMUNITY, &laptop, PERSON_RECORD, "o").expect("endorse laptop");
    let cert = GenesisCert {
        name: "c".into(),
        policy: rekindle_types::governance::AdmissionMode::Open,
        initial_roles: Vec::new(),
        creator_pseudonym: author.clone(),
        creator_member_record: PERSON_RECORD.into(),
        creator_member_record_owner: "o".into(),
        created_at: 1,
        additional_creators: Vec::new(),
        predecessor: None,
        creator_device: Some(creator_device),
    };
    let mut state = merge::merge(&cert, &[]);
    state.devices.seed_genesis(COMMUNITY, &cert);
    let phone = SigningKey::from_bytes(&PHONE_KEY)
        .verifying_key()
        .to_bytes();
    let record = DeviceRecord {
        record: PHONE_RECORD.into(),
        owner: "opd".into(),
    };
    let held = state.devices.member(&author);
    let e =
        endorse(&p, COMMUNITY, PERSON_RECORD, held, phone, record, [0; 32]).expect("endorse phone");
    state.devices.absorb(
        COMMUNITY,
        &GovernanceOp {
            author: author.clone(),
            seq: 0,
            prev: None,
            deps: Vec::new(),
            issued_at: 0,
            body: GovernanceEntry::DeviceKey(e),
            sig: [0; 64],
            device_key: phone,
        },
    );
    (author, cert, state)
}

/// The record and owner our frontier republish writes with, on a device
/// holding `device_key` (bound to `M(p,d)` when `phone`).
async fn frontier_target(device_key: [u8; 32], phone: bool) -> (String, String) {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let (me, cert, state) = two_device_state();
    if let Some(c1) = host.ctx.communities.map().write().get_mut(COMMUNITY) {
        c1.my_pseudonym_key = Some(me.to_hex());
        c1.genesis_cert = Some(cert);
    }
    host.ctx
        .communities
        .set_my_member_record(COMMUNITY, PERSON_RECORD);
    {
        let guard = host.ctx.vault.lock();
        let vault = guard.as_ref().expect("identity vault");
        let key = rekindle_vault::VaultKey::MemberRecordOwner {
            community: COMMUNITY.into(),
        };
        vault.put(&key, PERSON_OWNER.as_bytes()).unwrap();
    }
    {
        let guard = host.ctx.device_vault.lock();
        let vault = guard.as_ref().expect("device vault");
        rekindle_vault::typed::device_keys::store_device_op_key(vault, COMMUNITY, &device_key)
            .unwrap();
        if phone {
            rekindle_vault::typed::device_keys::store_device_record(
                vault,
                COMMUNITY,
                PHONE_RECORD,
                PHONE_OWNER,
            )
            .unwrap();
        }
    }
    let governance = HostGovernance::new(Arc::clone(&host.ctx));
    governance.set_governance_state(COMMUNITY, state);
    let (record, writer) = governance.own_op_record_writer(COMMUNITY).unwrap();
    let target = (record, writer.as_str().to_owned());
    host.shutdown().await.unwrap();
    target
}

#[tokio::test]
async fn a_second_devices_frontier_lands_on_its_own_record_with_its_own_owner() {
    assert_eq!(
        frontier_target(PHONE_KEY, true).await,
        (PHONE_RECORD.to_owned(), PHONE_OWNER.to_owned()),
        "a second device republishes on M(p,d) with K_M(p,d), never on M(p)"
    );
}

#[tokio::test]
async fn the_first_devices_frontier_stays_on_the_member_record() {
    assert_eq!(
        frontier_target(LAPTOP_KEY, false).await,
        (PERSON_RECORD.to_owned(), PERSON_OWNER.to_owned()),
    );
}

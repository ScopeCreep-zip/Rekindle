//! `in_the_cut` (plan E3.2 K5k): the cut index is keyed by device key (ADR
//! 0022 rule 1), so a joiner's op is found by the author its merged cut op
//! names, or by an unmerged key the snapshot's device set holds `p`'s
//! endorsement of; never by the pseudonym as a key.

use rekindle_governance::checkpoint::{CutIndex, CutOp, CutSnapshot};
use rekindle_governance::state::GovernanceState;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::{GovernanceEntry, GovernanceOp, OpHash};
use rekindle_types::id::PseudonymKey;

use super::in_the_cut;
use crate::device_signer::first_endorsement;

const ANCHOR: &str = "VLD0:anchor";
const DEVICE: [u8; 32] = [8; 32];

fn p() -> SigningKey {
    SigningKey::from_bytes(&[1; 32])
}

fn joiner() -> PseudonymKey {
    PseudonymKey(p().verifying_key().to_bytes())
}

fn device_key() -> [u8; 32] {
    SigningKey::from_bytes(&DEVICE).verifying_key().to_bytes()
}

/// A state whose device set holds `p`'s endorsement of `DEVICE`'s key.
fn endorsed_state() -> GovernanceState {
    let device = SigningKey::from_bytes(&DEVICE);
    let e = first_endorsement(&p(), ANCHOR, &device, "VLD0:m", "o").unwrap();
    let mut state = GovernanceState::default();
    state.devices.absorb(
        ANCHOR,
        &GovernanceOp {
            author: joiner(),
            seq: 0,
            prev: None,
            deps: Vec::new(),
            issued_at: 0,
            body: GovernanceEntry::MemberJoined {
                member_record: "VLD0:m".into(),
                member_record_owner: "o".into(),
                invite_proof: None,
                owner_proof: vec![0; 64],
                first_device: Some(e),
            },
            sig: [0; 64],
            device_key: device_key(),
        },
    );
    state
}

fn snap(state: GovernanceState, cut: CutIndex) -> CutSnapshot {
    CutSnapshot {
        state,
        cut,
        heads: Vec::new(),
    }
}

#[test]
fn a_merged_cut_op_is_matched_by_its_author_under_its_device_key() {
    let mut cut = CutIndex::default();
    let op = CutOp {
        hash: OpHash([4; 32]),
        author: joiner().0,
        device_key: device_key(),
        seq: 0,
    };
    cut.authors.insert(device_key(), op);
    assert!(in_the_cut(
        &snap(GovernanceState::default(), cut.clone()),
        &joiner()
    ));
    let stranger = PseudonymKey([9; 32]);
    assert!(!in_the_cut(
        &snap(GovernanceState::default(), cut),
        &stranger
    ));
}

#[test]
fn an_unmerged_key_is_matched_when_p_endorsed_it() {
    let mut cut = CutIndex::default();
    cut.unmerged.insert(device_key(), 3);
    assert!(in_the_cut(&snap(endorsed_state(), cut.clone()), &joiner()));
    // The same unmerged key with no endorsement of it held: not the joiner's.
    assert!(!in_the_cut(
        &snap(GovernanceState::default(), cut),
        &joiner()
    ));
}

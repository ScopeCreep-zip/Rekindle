use rekindle_codec::governance_op::{decode_op, encode_op, op_unsigned_bytes};
use rekindle_governance::device::retirement_signing_bytes;
use rekindle_governance::device_writer::{endorse, DeviceRecord};
use rekindle_governance::op::{op_hash, op_signing_bytes};
use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
use rekindle_types::governance::{
    DeviceReEnrol, DeviceRetired, GovernanceEntry, GovernanceOp, OpRef,
};

use super::verified_successor_owner;
use crate::op_log::OwnMember;
use crate::test_member::Member;

#[test]
fn a_retirement_pointer_requires_the_members_signed_retirement() {
    let original = Member::new(1);
    let successor = Member::new(2);
    let community = Member::new(3).record();
    let me = OwnMember {
        pseudonym: original.author(),
        hex: original.author().to_hex(),
        record: Some(original.record()),
    };
    let member_record = me.record.as_deref().unwrap();
    let old_key = original.device.verifying_key().to_bytes();
    let new_key = successor.device.verifying_key().to_bytes();
    let endorsement = endorse(
        &original.pseudonym,
        &community,
        member_record,
        None,
        new_key,
        DeviceRecord {
            record: successor.record(),
            owner: successor.owner_text(),
        },
        [0; 32],
    )
    .unwrap();
    let mut retirement = DeviceRetired {
        device_key: old_key,
        incarnation: 1,
        last_seq: 4,
        retirement_sig: Vec::new(),
    };
    let retirement_bytes = retirement_signing_bytes(&community, member_record, &retirement);
    let mut verify = |retirement_signer: &SigningKey| {
        retirement.retirement_sig = retirement_signer
            .sign(&retirement_bytes)
            .to_bytes()
            .to_vec();
        let mut op = GovernanceOp {
            author: me.pseudonym.clone(),
            device_key: new_key,
            seq: 0,
            prev: None,
            deps: Vec::new(),
            issued_at: 0,
            body: GovernanceEntry::DeviceReEnrol(DeviceReEnrol {
                retired: retirement.clone(),
                endorsement: endorsement.clone(),
            }),
            sig: [0; 64],
        };
        let unsigned = op_unsigned_bytes(&op).unwrap();
        op.sig = successor
            .device
            .sign(&op_signing_bytes(&community, &unsigned))
            .to_bytes();
        let locator = OpRef {
            op_hash: op_hash(&unsigned, &op.sig),
            author: me.pseudonym.clone(),
            device_key: new_key,
            author_seq: 0,
            author_record: successor.record(),
        };
        let decoded = decode_op(&encode_op(&op).unwrap()).unwrap();
        verified_successor_owner(&community, &me, &locator, &old_key, 1, &decoded)
            .map(str::to_owned)
    };
    assert_eq!(verify(&original.pseudonym), Some(successor.owner_text()));
    // The forged locator hashes to an otherwise correctly signed op, with
    // a valid successor endorsement. Only the member may sign its retirement.
    assert_eq!(verify(&successor.pseudonym), None);
}

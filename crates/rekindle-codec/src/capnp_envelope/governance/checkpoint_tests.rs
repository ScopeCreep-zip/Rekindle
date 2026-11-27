use rekindle_types::governance::{Checkpoint, GovernanceEntry, OpHash, OpRef};
use rekindle_types::id::PseudonymKey;

use super::super::{decode_governance_entry, encode_governance_entry};

fn op_ref(n: u8) -> OpRef {
    OpRef {
        op_hash: OpHash([n; 32]),
        device_key: [n; 32],
        author: PseudonymKey([n; 32]),
        author_record: format!("VLD0:{}", "r".repeat(43)),
        author_seq: u64::from(n) << 40,
    }
}

fn checkpoint(heads: u8, prev: bool, pages: usize) -> GovernanceEntry {
    GovernanceEntry::Checkpoint(Checkpoint {
        height: if prev { 2 } else { 1 },
        round: 3,
        prev: prev.then(|| op_ref(200)),
        cut: (0..heads).map(op_ref).collect(),
        frontier_hash: [1; 32],
        dir_root: [2; 32],
        dir_size: 128 * pages as u64,
        state_root: [3; 32],
        state_size: 5,
        mek_generation: 4,
        eligible_set_hash: [5; 32],
        pages: (0..pages).map(|p| format!("VLD0:page{p:039}")).collect(),
    })
}

#[test]
fn a_checkpoint_round_trips() {
    for entry in [checkpoint(1, false, 0), checkpoint(63, true, 16)] {
        let bytes = encode_governance_entry(&entry);
        assert_eq!(decode_governance_entry(&bytes.unwrap()).expect("decodes"), entry);
    }
}

#[test]
fn a_short_digest_is_refused() {
    let GovernanceEntry::Checkpoint(c) = checkpoint(1, false, 0) else {
        unreachable!("built as a checkpoint")
    };
    let mut msg = capnp::message::Builder::new_default();
    let mut b = msg.init_root::<super::checkpoint_entry::Builder<'_>>();
    super::write_checkpoint(b.reborrow(), &c);
    assert!(super::read_checkpoint(b.reborrow_as_reader()).is_ok());
    b.set_state_root(&[3; 31]);
    assert!(super::read_checkpoint(b.into_reader()).is_err());
}

/// The largest `Checkpoint` op: 64 deps, 63 of them its cut heads, a
/// previous checkpoint and 64 page locators (8,192 members).
fn largest_checkpoint_op() -> rekindle_types::governance::GovernanceOp {
    use crate::governance_op::GOVERNANCE_OP_MAX_DEPS;
    let deps: Vec<OpRef> = (0..GOVERNANCE_OP_MAX_DEPS)
        .map(|i| op_ref(u8::try_from(i).unwrap_or(u8::MAX)))
        .collect();
    let GovernanceEntry::Checkpoint(mut c) = checkpoint(0, true, 64) else {
        unreachable!("built as a checkpoint")
    };
    c.cut = deps[1..].to_vec();
    rekindle_types::governance::GovernanceOp {
        device_key: [0xA0; 32],
        author: PseudonymKey([0xA0; 32]),
        seq: u64::MAX,
        prev: Some(OpHash([0xB0; 32])),
        deps,
        issued_at: u64::MAX,
        body: GovernanceEntry::Checkpoint(c),
        sig: [0xC0; 64],
    }
}

/// The largest `Checkpoint` op fits one governance op value.
#[test]
fn the_largest_checkpoint_op_fits() {
    use crate::governance_op::{decode_op, encode_op, GOVERNANCE_OP_MAX_BYTES};
    let op = largest_checkpoint_op();
    let bytes = encode_op(&op).expect("encodes");
    assert!(
        bytes.len() <= GOVERNANCE_OP_MAX_BYTES,
        "{} bytes",
        bytes.len()
    );
    assert_eq!(decode_op(&bytes).expect("decodes").op, op);
}

/// Plan E3.2 S7c.8 item (5): the largest `Checkpoint` op also rides the
/// gossip fast path (`governanceOp @74`): in its community envelope,
/// signed into the `SignedEnvelope` the mesh sends, under the longest
/// community id and a hex pseudonym, it fits Veilid's 32,768 B
/// `app_message` cap (`veilid-core/proto/veilid.capnp:33-34`).
#[test]
fn the_largest_checkpoint_op_fits_a_gossiped_app_message() {
    use crate::capnp_envelope::{encode_community_envelope, encode_signed_envelope};
    use crate::community::envelope::{sign_envelope, CommunityEnvelope, ControlPayload};
    use rekindle_types::dht_layout::MAX_APP_MESSAGE_LEN;
    let op = crate::governance_op::encode_op(&largest_checkpoint_op()).expect("encodes");
    let envelope =
        encode_community_envelope(&CommunityEnvelope::control(ControlPayload::GovernanceOp {
            op,
        }))
        .expect("encodes");
    let key = rekindle_secrets::ed25519_dalek::SigningKey::from_bytes(&[9; 32]);
    let community = format!("VLD0:{}", "A".repeat(150));
    let signed = sign_envelope(&key, &community, &"ab".repeat(32), [7u8; 16], &envelope);
    let wire = encode_signed_envelope(&signed).unwrap();
    assert!(
        wire.len() <= MAX_APP_MESSAGE_LEN,
        "{} B over the {MAX_APP_MESSAGE_LEN} B app_message cap",
        wire.len()
    );
}

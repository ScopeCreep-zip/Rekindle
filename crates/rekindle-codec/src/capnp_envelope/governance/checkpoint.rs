//! The governance `Checkpoint` op body (plan E3.2 S7c.3).

use crate::capnp_codec::{capnp_err, text_to_string};
use crate::community_governance_capnp::checkpoint_entry;
use crate::error::CodecError;
use crate::governance_op::{read_op_ref, write_op_ref};
use rekindle_types::governance::{Checkpoint, GovernanceEntry};

pub(super) fn read_checkpoint(
    p: checkpoint_entry::Reader<'_>,
) -> Result<GovernanceEntry, CodecError> {
    let prev = if p.get_has_prev_checkpoint() {
        Some(read_op_ref(
            p.get_prev_checkpoint().map_err(|e| capnp_err(&e))?,
        )?)
    } else {
        None
    };
    let cut = (p.get_cut().map_err(|e| capnp_err(&e))?.iter())
        .map(read_op_ref)
        .collect::<Result<Vec<_>, _>>()?;
    let pages = (p.get_pages().map_err(|e| capnp_err(&e))?.iter())
        .map(|t| text_to_string(t.map_err(|e| capnp_err(&e))?))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(GovernanceEntry::Checkpoint(Checkpoint {
        height: p.get_height(),
        round: p.get_round(),
        prev,
        cut,
        frontier_hash: hash32(p.get_frontier_hash(), "frontierHash")?,
        dir_root: hash32(p.get_dir_root(), "dirRoot")?,
        dir_size: p.get_dir_size(),
        state_root: hash32(p.get_state_root(), "stateRoot")?,
        state_size: p.get_state_size(),
        mek_generation: p.get_mek_generation(),
        eligible_set_hash: hash32(p.get_eligible_set_hash(), "eligibleSetHash")?,
        pages,
    }))
}

pub(super) fn write_checkpoint(mut p: checkpoint_entry::Builder<'_>, c: &Checkpoint) {
    p.set_height(c.height);
    p.set_round(c.round);
    if let Some(prev) = &c.prev {
        p.set_has_prev_checkpoint(true);
        write_op_ref(p.reborrow().init_prev_checkpoint(), prev);
    }
    let mut cut = p.reborrow().init_cut(len(c.cut.len()));
    for (i, head) in c.cut.iter().enumerate() {
        write_op_ref(cut.reborrow().get(len(i)), head);
    }
    p.set_frontier_hash(&c.frontier_hash);
    p.set_dir_root(&c.dir_root);
    p.set_dir_size(c.dir_size);
    p.set_state_root(&c.state_root);
    p.set_state_size(c.state_size);
    p.set_mek_generation(c.mek_generation);
    p.set_eligible_set_hash(&c.eligible_set_hash);
    let mut pages = p.init_pages(len(c.pages.len()));
    for (i, page) in c.pages.iter().enumerate() {
        pages.set(len(i), page.as_str());
    }
}

fn len(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// A 32-byte digest field; any other length is refused.
fn hash32(get: Result<&[u8], capnp::Error>, what: &str) -> Result<[u8; 32], CodecError> {
    let bytes = get.map_err(|e| capnp_err(&e))?;
    bytes.try_into().map_err(|_| {
        CodecError::Deserialization(format!(
            "checkpoint {what} is {} bytes, not 32",
            bytes.len()
        ))
    })
}

#[cfg(test)]
mod tests {
    use rekindle_types::governance::{Checkpoint, GovernanceEntry, OpHash, OpRef};
    use rekindle_types::id::PseudonymKey;

    use super::super::{decode_governance_entry, encode_governance_entry};

    fn op_ref(n: u8) -> OpRef {
        OpRef {
            op_hash: OpHash([n; 32]),
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
            assert_eq!(decode_governance_entry(&bytes).expect("decodes"), entry);
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
            encode_community_envelope(&CommunityEnvelope::Control(ControlPayload::GovernanceOp {
                op,
            }))
            .expect("encodes");
        let key = rekindle_secrets::ed25519_dalek::SigningKey::from_bytes(&[9; 32]);
        let community = format!("VLD0:{}", "A".repeat(150));
        let signed = sign_envelope(&key, &community, &"ab".repeat(32), [7u8; 16], &envelope);
        let wire = encode_signed_envelope(&signed);
        assert!(
            wire.len() <= MAX_APP_MESSAGE_LEN,
            "{} B over the {MAX_APP_MESSAGE_LEN} B app_message cap",
            wire.len()
        );
    }
}

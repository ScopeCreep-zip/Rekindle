//! The walk-cost DAG (plan S7c.1), shared by `src/op_walk_tests.rs` and
//! `tests/walk_cpu_budget.rs`: the creator's two ops, then 64 authors with
//! two ops each, the first citing the creator's second op: a 64-head level
//! twice over (the shallow-join shape, S7c.0), walked from the 64 heads.

use std::collections::HashMap;

use rekindle_codec::governance_op::{encode_op, op_unsigned_bytes};
use rekindle_governance::fixture::meta;
use rekindle_governance::op::{op_hash, op_signing_bytes};
use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
use rekindle_types::governance::{GovernanceOp, OpRef};
use rekindle_types::id::PseudonymKey;

pub const ANCHOR: &str = "A";
pub const AUTHORS: usize = 64;

/// Each author record's op log; chain seq = index + 1.
pub struct Dag {
    pub logs: HashMap<String, Vec<Vec<u8>>>,
    pub heads: Vec<OpRef>,
    pub ops: usize,
}

impl Dag {
    pub fn encoded(&self, op_ref: &OpRef) -> Option<Vec<u8>> {
        let index = usize::try_from(op_ref.author_seq).ok()?;
        self.logs.get(&op_ref.author_record)?.get(index).cloned()
    }
}

/// An author's chain of `len` ops in `record`, the first citing `first_deps`.
fn chain(seed: u8, record: &str, len: u64, first_deps: &[OpRef]) -> (Vec<Vec<u8>>, OpRef) {
    let key = SigningKey::from_bytes(&[seed; 32]);
    let author = PseudonymKey(key.verifying_key().to_bytes());
    let mut log = Vec::new();
    let mut last: Option<OpRef> = None;
    for seq in 0..len {
        let deps = last
            .clone()
            .map_or_else(|| first_deps.to_vec(), |prev| vec![prev]);
        let mut op = GovernanceOp {
            author: author.clone(),
            seq,
            prev: last.as_ref().map(|r| r.op_hash),
            deps,
            issued_at: 10 + seq,
            body: meta("m"),
            sig: [0; 64],
        };
        let unsigned = op_unsigned_bytes(&op);
        op.sig = key.sign(&op_signing_bytes(ANCHOR, &unsigned)).to_bytes();
        let hash = op_hash(&unsigned, &op.sig);
        log.push(encode_op(&op).unwrap());
        last = Some(OpRef {
            op_hash: hash,
            author: author.clone(),
            author_record: record.to_owned(),
            author_seq: seq,
        });
    }
    (log, last.unwrap())
}

pub fn dag() -> Dag {
    let mut logs = HashMap::new();
    let (creator_log, creator_head) = chain(1, "rec-creator", 2, &[]);
    logs.insert("rec-creator".to_owned(), creator_log);
    let mut heads = Vec::new();
    for i in 0..AUTHORS {
        let record = format!("rec-{i}");
        let seed = u8::try_from(i + 2).unwrap();
        let (log, head) = chain(seed, &record, 2, std::slice::from_ref(&creator_head));
        logs.insert(record, log);
        heads.push(head);
    }
    Dag {
        ops: 2 + 2 * AUTHORS,
        logs,
        heads,
    }
}

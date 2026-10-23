//! Stored governance for the Host tests (plan E3.2): signed op chains and
//! a pinned cert, seeded through the Host's op store as the warm restore
//! reads them.

use rekindle_secrets::ed25519_dalek::SigningKey;

use crate::host::Host;

/// The member record the fixture cert names for its creator.
pub(super) const CREATOR_RECORD: &str = "VLD0:creator-record";

pub(super) fn meta(name: &str) -> rekindle_types::governance::GovernanceEntry {
    rekindle_types::governance::GovernanceEntry::CommunityMeta {
        name: Some(name.into()),
        description: None,
        icon_hash: None,
        banner_hash: None,
    }
}

/// `key`'s op chain in community `anchor`, signed as production signs
/// it (the restore verifies it): each op cites the one before it, and the
/// chain lives in `record`.
pub(super) fn signed_chain(
    anchor: &str,
    key: &SigningKey,
    record: &str,
    bodies: Vec<rekindle_types::governance::GovernanceEntry>,
) -> Vec<rekindle_governance_runtime::op_log::HeldOp> {
    use rekindle_codec::governance_op::{encode_op, op_unsigned_bytes};
    use rekindle_governance::op::{op_hash, op_signing_bytes};
    use rekindle_types::governance::{GovernanceOp, OpRef};

    let author = rekindle_types::id::PseudonymKey(key.verifying_key().to_bytes());
    let mut out: Vec<rekindle_governance_runtime::op_log::HeldOp> = Vec::new();
    for (seq, body) in (0u64..).zip(bodies) {
        let prev = out.last().map(|p| (p.hash, p.seq));
        let mut op = GovernanceOp {
            author: author.clone(),
            seq,
            prev: prev.map(|(hash, _)| hash),
            deps: prev
                .map(|(op_hash, author_seq)| OpRef {
                    op_hash,
                    author: author.clone(),
                    author_record: record.into(),
                    author_seq,
                })
                .into_iter()
                .collect(),
            issued_at: seq + 1,
            body,
            sig: [0; 64],
        };
        let unsigned = op_unsigned_bytes(&op);
        op.sig = rekindle_secrets::derive::sign_with_pseudonym(
            key,
            &op_signing_bytes(anchor, &unsigned),
        );
        out.push(rekindle_governance_runtime::op_log::HeldOp {
            hash: op_hash(&unsigned, &op.sig),
            author: author.clone(),
            author_record: record.into(),
            seq,
            lamport: seq + 1,
            issued_at: seq + 1,
            encoded: encode_op(&op).unwrap(),
            fate: rekindle_governance_runtime::op_log::OpFate::Applied,
        });
    }
    out
}

/// Pin a cert naming `creator` for Ann's stored `community` and store
/// `ops` through the Host's op store, as the warm restore reads them.
pub(super) async fn seed_governance(
    host: &Host,
    owner: &str,
    community: &str,
    creator: &SigningKey,
    ops: &[rekindle_governance_runtime::op_log::HeldOp],
) -> rekindle_types::governance::GenesisCert {
    use rekindle_codec::governance_op::genesis::{encode_genesis, SignedGenesis};
    use rekindle_governance_runtime::GovernanceRuntimeDeps;

    let cert = rekindle_governance::fixture::cert(
        &rekindle_types::id::PseudonymKey(creator.verifying_key().to_bytes()),
        rekindle_types::governance::AdmissionMode::default(),
    );
    let bytes = encode_genesis(&SignedGenesis {
        cert: cert.clone(),
        anchor_owner: [0; 32],
        anchor_sig: [0; 64],
        creator_sig: [0; 64],
    })
    .unwrap();
    let (o, c) = (owner.to_string(), community.to_string());
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| {
            rekindle_db::repo::communities::set(
                conn,
                &o,
                &c,
                rekindle_db::repo::communities::Column::GenesisCert,
                bytes,
            )
        })
        .await
        .unwrap();
    for op in ops {
        host.governance()
            .store_governance_op(community, op)
            .await
            .unwrap();
    }
    cert
}

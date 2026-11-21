//! The joiner's walk over a 64-head level (plan E3.2 S7c.1): 64 authors'
//! op logs on member records, each first op citing the creator's head (the
//! shallow-join shape, S7c.0), walked from the 64 heads by another node
//! through the production pool and `op_walk::fetch_from_log`, the read the
//! Host's `fetch_governance_op` runs. The level's fetches run sixteen at a
//! time and each author log is read once, so the walk costs about four
//! single-fetch round trips, where a fetch at a time over a re-read log
//! took 130.

use std::time::Duration;

use rekindle_codec::governance_op::{encode_op, op_unsigned_bytes};
use rekindle_governance::op::{op_hash, op_signing_bytes};
use rekindle_governance_runtime::op_log::held_from_encoded;
use rekindle_governance_runtime::op_walk::{
    fetch_from_log, walk_with, WalkLogCache, WALK_FETCHES_IN_FLIGHT,
};
use rekindle_governance_runtime::verify_cache::OpVerifyCache;
use rekindle_netsim::Netsim;
use rekindle_records::port::RecordPort;
use rekindle_records::schema;
use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
use rekindle_types::dht_layout::member_record::GOV_HEAD;
use rekindle_types::governance::{GovernanceEntry, GovernanceOp, OpRef};
use rekindle_types::id::PseudonymKey;
use tokio::time::Instant;

const ANCHOR: &str = "walk-cost-anchor";
const AUTHORS: u8 = 64;

/// A member record of `seed`'s key holding a two-op chain, the first op
/// citing `first_deps`; its head's `OpRef`.
async fn author(port: &dyn RecordPort, seed: u8, first_deps: &[OpRef]) -> OpRef {
    let signing = SigningKey::from_bytes(&[seed; 32]);
    let writer = rekindle_crypto::keypair_text::keypair_text_from_signing(&signing);
    let (lease, record, _) = port
        .create(&schema::member_record(), Some(&writer))
        .await
        .expect("create the member record");
    port.release(lease).await;
    let author = PseudonymKey(signing.verifying_key().to_bytes());
    let mut entries = Vec::new();
    let mut last: Option<OpRef> = None;
    for seq in 0..2 {
        let mut op = GovernanceOp {
            device_key: author.0,
            author: author.clone(),
            seq,
            prev: last.as_ref().map(|r| r.op_hash),
            deps: last
                .clone()
                .map_or_else(|| first_deps.to_vec(), |prev| vec![prev]),
            issued_at: 10 + seq,
            body: GovernanceEntry::CommunityMeta {
                name: Some(format!("m{seed}")),
                description: None,
                icon_hash: None,
                banner_hash: None,
            },
            sig: [0; 64],
        };
        let unsigned = op_unsigned_bytes(&op);
        op.sig = signing
            .sign(&op_signing_bytes(ANCHOR, &unsigned))
            .to_bytes();
        entries.push(encode_op(&op).expect("encode"));
        last = Some(OpRef {
            op_hash: op_hash(&unsigned, &op.sig),
            device_key: author.0,
            author: author.clone(),
            author_record: record.clone(),
            author_seq: seq,
        });
    }
    rekindle_records::chain::append_at(port, &record, GOV_HEAD, &writer, &entries)
        .await
        .expect("append the op log");
    last.expect("a head")
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_sixty_four_head_level_costs_one_round_trip_bound() {
    let (_sim, nodes) = Netsim::nodes(2, 4064);
    let (writer, reader) = (
        nodes[0].ports.records.start_session().unwrap(),
        nodes[1].ports.records.start_session().unwrap(),
    );
    let creator = author(writer.as_ref(), 1, &[]).await;
    let mut heads = Vec::new();
    for seed in 2..2 + AUTHORS {
        heads.push(author(writer.as_ref(), seed, std::slice::from_ref(&creator)).await);
    }
    // One fetch's cost on the reader: a log of the same shape it has not
    // opened, read cold.
    let probe = author(writer.as_ref(), 200, std::slice::from_ref(&creator)).await;
    let started = Instant::now();
    let fetched = fetch_from_log(reader.as_ref(), &WalkLogCache::default(), &probe)
        .await
        .unwrap();
    let one_fetch = started.elapsed();
    assert!(fetched.is_some() && one_fetch > Duration::ZERO);

    let logs = WalkLogCache::default();
    let (port, logs_ref) = (reader.as_ref(), &logs);
    let started = Instant::now();
    let walked = walk_with(
        ANCHOR,
        &heads,
        &OpVerifyCache::default(),
        |op_ref| async move {
            Ok(fetch_from_log(port, logs_ref, &op_ref)
                .await?
                .and_then(|encoded| held_from_encoded(&op_ref, encoded)))
        },
    )
    .await
    .unwrap();
    let walk = started.elapsed();
    assert_eq!(walked.len(), 2 + 2 * usize::from(AUTHORS));
    // The heads' level reads 64 logs sixteen at a time; every lower level
    // is served from the logs already read, but the creator's, read once:
    // five single-fetch round trips (measured 10.83 s against one fetch of
    // 2.16 s), with half a round trip of slack for the replicas' jitter.
    let rounds = u32::try_from(usize::from(AUTHORS).div_ceil(WALK_FETCHES_IN_FLIGHT) + 1).unwrap();
    let bound = one_fetch * rounds + one_fetch / 2;
    assert!(
        walk <= bound,
        "the walk took {walk:?}; one fetch {one_fetch:?}, bound {bound:?}"
    );
}

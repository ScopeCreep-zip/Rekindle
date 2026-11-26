//! The local signed log (plan E3.2 S7d-keys.3, ADR 0022 rule 4): an op is
//! `signed_here` only when this device's signer stored it, never when it
//! arrived by gossip, the DAG fetch or a bootstrap, which self-detection
//! (S7d-keys.9) trusts.

use std::collections::BTreeMap;

use rekindle_governance_runtime::deps::GovernanceOpStore as _;
use rekindle_secrets::ed25519_dalek::SigningKey;

use super::{
    create_ann, meta, offline_host, seed_governance, seed_registry_community, signed_chain,
    CREATOR_RECORD,
};

#[tokio::test]
async fn an_ingested_op_never_sets_signed_here() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let ann = create_ann(&host).await;
    seed_registry_community(&host, &ann.public_key).await;
    let key = SigningKey::from_bytes(&[0xbb; 32]);
    let ops = signed_chain(
        "c1",
        &key,
        CREATOR_RECORD,
        vec![meta("A"), meta("B"), meta("C"), meta("D")],
    );
    // The DAG fetch's and the bootstrap's store.
    let cert = seed_governance(&host, &ann.public_key, "c1", &key, &ops[..1]).await;
    host.ctx
        .communities
        .map()
        .write()
        .get_mut("c1")
        .unwrap()
        .genesis_cert = Some(cert);
    let gov = host.governance();
    // A read at its locator, which a gossip triggers (lead ruling
    // 2026-11-26 item 1: a gossiped op is stored only as that read
    // returns it; this host is offline, so the read's receive is fed).
    assert!(
        rekindle_governance_runtime::op_log::receive_op(&gov, "c1", ops[1].encoded.clone())
            .await
            .unwrap(),
        "the op read at its locator is stored"
    );
    // Our own signer, then our op echoed back by gossip.
    gov.store_signed_governance_op("c1", &ops[2]).await.unwrap();
    gov.on_governance_op("c1", ops[2].encoded.clone()).await;
    // The ingest store again, for an op no signer here wrote.
    gov.store_governance_op("c1", &ops[3]).await.unwrap();

    let owner = ann.public_key.clone();
    let flags: BTreeMap<[u8; 32], bool> = host
        .ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| rekindle_db::repo::governance_ops::list(conn, &owner, "c1", None))
        .await
        .unwrap()
        .into_iter()
        .map(|op| (op.op_hash, op.signed_here))
        .collect();
    let want: BTreeMap<[u8; 32], bool> = ops
        .iter()
        .enumerate()
        .map(|(i, op)| (op.hash.0, i == 2))
        .collect();
    assert_eq!(flags, want, "only the op our signer stored is signed_here");
    host.shutdown().await.unwrap();
}

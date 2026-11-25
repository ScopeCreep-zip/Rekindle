//! `community info` serves where our governance ops not yet landed stand,
//! from the same Host function the window's view reads (one source: the
//! CLI and TUI get exactly the window's state).

use std::sync::Arc;

use rekindle_secrets::ed25519_dalek::SigningKey;

use super::own_removal::data;
use super::{
    create_ann, meta, offline_host, seed_registry_community, signed_chain, CREATOR_RECORD,
};
use crate::daemon::dispatch::community::handle_info;
use crate::daemon::DaemonState;
use crate::host::adapters::governance::HostGovernance;

#[tokio::test]
async fn info_serves_the_windows_op_publications_until_the_op_lands() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    super::onboarding_bounds::load_membership(&host);
    let key = SigningKey::from_bytes(&[0xbb; 32]);
    let op = signed_chain("c1", &key, CREATOR_RECORD, vec![meta("C")]).remove(0);
    let ops = (host.ctx.outbox.governance_ops()).expect("the unlocked Host runs its outbox");
    ops.queue("c1", CREATOR_RECORD, op.encoded.clone())
        .await
        .unwrap();

    let info = data(handle_info(&host.ctx, DaemonState::Operational, "c1").await);
    let pending = (info["op_publications"].as_object())
        .expect("info carries op_publications")
        .clone();
    assert_eq!(pending.len(), 1, "the queued op stands: {info}");
    let (id, state) = pending.iter().next().unwrap();
    assert_eq!(id, &hex::encode(op.hash.0));
    assert_eq!(state, &serde_json::json!({ "kind": "pending" }));
    let views = HostGovernance::new(Arc::clone(&host.ctx))
        .community_views()
        .await;
    assert_eq!(
        serde_json::to_value(&views[0].op_publications).unwrap(),
        serde_json::Value::Object(pending.clone()),
        "info and the window read one state"
    );

    ops.settle_mirrored("c1", std::slice::from_ref(id))
        .await
        .unwrap();
    let info = data(handle_info(&host.ctx, DaemonState::Operational, "c1").await);
    assert_eq!(
        info["op_publications"],
        serde_json::json!({}),
        "a landed op leaves"
    );
    host.shutdown().await.unwrap();
}

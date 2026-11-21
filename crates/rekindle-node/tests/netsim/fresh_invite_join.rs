//! A join through an invite minted right after the community was created
//! (live walk B60 → B61, finding IV1: on Linux 1 in 4 such joins was refused
//! 404 "This invite is not one the community knows"). The inviter hands the
//! link out only once its `InviteCreated` and its frontier are on `M`/2 and
//! `M`/3 (`apply::write_invite_created`, plan E3.2 S6 (4)), so a joiner that
//! walks from that frontier at once, with no settle wait, finds the invite
//! (`join_flow::check_invite`) on every seed.
//!
//! The seed set is `0..IV1_SEEDS` (default 8); a sweep sets it higher. Every
//! refused seed is collected with its trace hash before the assertion, so one
//! run names them all.

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};

use crate::harness::Community;

fn seeds() -> u64 {
    std::env::var("IV1_SEEDS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(8)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_fresh_invite_is_known_to_an_immediate_join() {
    let mut refused = Vec::new();
    for seed in 0..seeds() {
        let mut community = Community::create(2, 9_100 + seed).await;
        let link = community.invite().await;
        let joined = community.nodes[1]
            .call(IpcRequest::Community(CommunityRequest::Lifecycle(
                LifecycleRequest::CommunityJoin { invite: link },
            )))
            .await;
        if let Err(refusal) = joined {
            let hash = hex(&community.sim.trace().hash());
            eprintln!(
                "iv1: seed {} refused {} {:?} trace {hash}",
                9_100 + seed,
                refusal.code,
                refusal.message
            );
            refused.push((9_100 + seed, refusal.code, refusal.message, hash));
        }
    }
    assert!(
        refused.is_empty(),
        "joins refused through a fresh invite: {refused:?}"
    );
}

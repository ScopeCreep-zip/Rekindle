//! Record warming of sealed pages (plan E3.2 S7, ADR 0011 "Sealed pages",
//! architecture §14.1 mutual aid). The creator proposes the height-1
//! checkpoint, which its own endorsement makes usable (S7c.4), and seals
//! its directory pages; a member that did not propose holds them from the
//! endorsed checkpoint. The creator goes offline and other traffic turns
//! every replica store over (0.5.8 stores are an LRU of
//! `remote_max_records` = 128, `veilid_config.rs:356`): the member's
//! keepalive re-opens each page with `Hold`, which re-pushes the stored
//! signed value (`rehydrate.rs:309-330`), so a node that never read a page
//! still reads every one from the network.

use std::time::Duration;

use rekindle_governance::checkpoint::directory::DirectoryPage;
use rekindle_netsim::Fault;

use rekindle_governance_runtime::deps::GovernanceRuntimeDeps;
use rekindle_ipc::protocol::{ChannelRequest, CommunityRequest, IpcRequest};
use rekindle_types::dht_layout::member_record::GOV_HEAD;

use crate::harness::{Community, PASSPHRASE};
use crate::watch_ring::creator_record;

/// Two keepalive windows: 300 s plus up to 60 s of jitter each
/// (`rekindle_record_pool::keepalive`).
const TWO_WINDOWS: Duration = Duration::from_secs(2 * 360);

/// How long a restarted node's process is down.
const RESTART_DOWN: Duration = Duration::from_secs(10);

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_member_keeps_the_checkpoint_directory_alive_after_its_author_leaves() {
    let mut community = Community::create(3, 4471).await;
    community.join(1).await;
    // The join is the last op; the proposal follows a quiet sweep.
    tokio::time::sleep(rekindle_governance_runtime::anti_entropy::SWEEP_PERIOD * 3).await;
    let pages: Vec<String> = (community.sim.single_subkey_values().into_iter())
        .filter(|(key, value)| {
            community.sim.creator(key) == Some(0) && DirectoryPage::decode(value).is_ok()
        })
        .map(|(key, _)| key)
        .collect();
    assert!(!pages.is_empty(), "the creator sealed the directory");
    // The member's keepalive reads each page once and holds it.
    tokio::time::sleep(TWO_WINDOWS).await;

    community.sim.fault(Fault::Detach {
        node: 0,
        detached: true,
    });
    let made = community.sim.store_traffic(2).await;
    assert!(made > 0, "the stores turned over");
    tokio::time::sleep(TWO_WINDOWS).await;

    for page in &pages {
        assert!(
            community.sim.network_get(page, 0).is_some(),
            "directory page {page} is readable after {made} other records"
        );
    }
}

/// The warm set persists nothing (S7 design step 3): after a restart and an
/// unlock the creator's login stage re-learns its own op pages beyond the
/// endorsed checkpoint's cut from its local op log, so its keepalive still
/// re-pushes its newest page while other traffic turns every store over.
/// Only after a restart does that re-push reach the network: Veilid 0.5.8
/// never rehydrates a record the node created in the current session
/// (`open_record.rs:62-74`), a set it forgets only at termination
/// (`storage_manager/mod.rs:681-695`), and a Lock keeps the transport.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_author_keeps_its_own_pages_warm_across_a_restart() {
    let mut community = Community::create(3, 4473).await;
    community.join(1).await;
    // The height-1 checkpoint folds the ops so far; this op is beyond it.
    tokio::time::sleep(rekindle_governance_runtime::anti_entropy::SWEEP_PERIOD * 3).await;
    let creator = &community.nodes[0];
    creator
        .ok(IpcRequest::Community(CommunityRequest::Channels(
            ChannelRequest::ChannelCreate {
                community: community.anchor.clone(),
                name: "lobby".into(),
                kind: "text".into(),
                category: None,
                topic: None,
                slowmode_seconds: 0,
                parent_voice_channel: None,
            },
        )))
        .await;
    tokio::time::sleep(Duration::from_secs(30)).await;
    let port = creator
        .host
        .governance()
        .record_port()
        .expect("a record port");
    let newest = rekindle_records::chain::head_at(&*port, &creator_record(&community), GOV_HEAD)
        .await
        .expect("the creator's op log head")
        .expect("the creator appended");
    // The old session's port goes with its Host.
    drop(port);

    let key = creator.public_key.clone().expect("an identity");
    let restarted = community
        .nodes
        .remove(0)
        .restart(&community.sim, RESTART_DOWN)
        .await;
    community.nodes.insert(0, restarted);
    community.nodes[0]
        .ok(IpcRequest::Unlock {
            identity: key,
            passphrase: PASSPHRASE.into(),
        })
        .await;
    tokio::time::sleep(TWO_WINDOWS).await;
    let made = community.sim.store_traffic(2).await;
    assert!(made > 0, "the stores turned over");
    tokio::time::sleep(TWO_WINDOWS).await;
    assert!(
        community.sim.network_get(&newest.page_key, 0).is_some(),
        "the creator's newest op page (seqs {}..={}) is readable after {made} other records",
        newest.first_seq,
        newest.last_seq
    );
}

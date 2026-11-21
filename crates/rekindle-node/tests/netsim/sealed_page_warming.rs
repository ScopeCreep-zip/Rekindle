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

use crate::harness::Community;

/// Two keepalive windows: 300 s plus up to 60 s of jitter each
/// (`rekindle_record_pool::keepalive`).
const TWO_WINDOWS: Duration = Duration::from_secs(2 * 360);

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

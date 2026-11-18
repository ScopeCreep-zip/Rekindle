//! A join from a checkpoint hint (plan E3.2 S7c.5, "Using a checkpoint"):
//! the creator's genesis holds power ops, so one `SWEEP_PERIOD` after its
//! last op it proposes the height-1 checkpoint, which its own endorsement
//! makes usable (the cert's creator set is the height-1 electorate, S7c.4).
//! A joiner then asks the creator, its inviter, for the hint by the
//! invite's code hash, checks the endorsement chain, the pages and the
//! snapshot, walks only above the cut and merges from the snapshot; the
//! join completes as a full walk's would (it lists the community and holds
//! its key).

use crate::harness::Community;

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_joiner_resumes_from_its_inviters_checkpoint() {
    let mut community = Community::create(2, 211).await;
    let link = community.invite().await;
    // The invite is the last op; the proposal follows a quiet sweep.
    tokio::time::sleep(rekindle_governance_runtime::anti_entropy::SWEEP_PERIOD * 3).await;
    community.join_with(1, link).await;
    let resumed = crate::logs::logged(1, &["join resumed from a checkpoint"]);
    assert_eq!(
        resumed.len(),
        1,
        "the join resumed from the hint: {resumed:?}"
    );
    assert!(crate::logs::warned("roster checksum").is_empty());
}

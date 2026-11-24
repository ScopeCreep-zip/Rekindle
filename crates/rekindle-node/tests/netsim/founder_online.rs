//! Plan E3.4 S3 and its premise check, finding F2: a founder never
//! announces a join, so the gossip mesh never knew her online to a joiner,
//! and the member list showed her offline while she posted. The list's
//! status for every other member is what her beats show, so the joiner's
//! list shows the founder online within one beat period of joining.

use std::time::Duration;

use crate::harness::{eventually, Community};
use crate::moderation::listed;

/// One beat period (`BEAT_PERIOD_MS`, 5 minutes).
const BEAT_PERIOD: Duration = Duration::from_secs(5 * 60);

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_joiner_lists_the_founder_online_from_her_beats() {
    let community = Community::of(2, 4410).await;
    let founder = community.pseudonym(0).await;
    let shown = std::cell::RefCell::new(None);
    let online = eventually(&community.sim, BEAT_PERIOD, || async {
        let row = listed(&community, 1, &founder).await;
        let online = row.as_ref().and_then(|m| m["status"].as_str()) == Some("online");
        *shown.borrow_mut() = row;
        online
    })
    .await;
    assert!(
        online,
        "the joiner lists the founder online within {BEAT_PERIOD:?}: {:?}",
        shown.borrow()
    );
    let row = shown.into_inner().expect("the founder is listed");
    assert_ne!(
        row["group"]["kind"], "offline",
        "the founder is grouped as present: {row}"
    );
}

//! Finding CB1 (Linux live at `ddfc1e70b`): a member who joins reads every
//! current member's channel outbox once (the join's catch-up, plan E3.3
//! "Cursor and authors"), so a post made before it joined shows in its
//! history without a further post from the author. Live, the joiner first
//! read the author's outbox two minutes later, on the author's next post's
//! hint.
//!
//! The joiner's catch-up starts before its first community key lands (the
//! invite carries none, D11; the inviter pushes it once it merged our
//! `MemberJoined`, D16), so the entry waits for the key; the key's install
//! starts another catch-up, which must not be lost to the one in flight.
//! The author stays a member and posts nothing more, and no time passes
//! before the check beyond what the join itself takes.
//!
//! A regression guard only: it passes with and without the CB1 fix (the
//! install's catch-up does not deterministically land during the join's
//! read here); the deterministic test is `rekindle-sync`'s
//! `a_trigger_during_a_pass_held_pending_reads_again_and_holds_the_entry`.
//! It adds what `outbox_catch_up` leaves out: a current author (not a
//! departed one) and a bound of one chain read, not a later post's hint.

use std::time::Duration;

use crate::harness::Community;

/// The bound: a catch-up's chain read, not a later post's hint.
const LIMIT: Duration = Duration::from_secs(30);

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_joiner_reads_a_current_members_earlier_post_on_join() {
    let mut community = Community::create(2, 1161).await;
    community
        .post(0, "posted before you joined")
        .await
        .expect("the creator posts");
    tokio::time::sleep(Duration::from_secs(5)).await;
    community.join(1).await;
    assert!(
        community
            .history_holds(1, "posted before you joined", LIMIT)
            .await,
        "the joiner's catch-up reads the creator's outbox"
    );
}

//! Plan E3.2 S7 Verify: with N = 40 members every member record `M` is
//! watched by its k = 4 ring predecessors and nobody else, under Veilid's
//! `public_watch_limit` (32), and no replica refuses a watch.

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn forty_members_watch_each_member_record_four_times_and_none_is_refused() {
    crate::watch_ring::ring_converges(40, 4040).await;
}

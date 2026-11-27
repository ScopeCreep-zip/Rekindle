use super::*;

#[tokio::test(start_paused = true)]
async fn idle_time_follows_the_simulated_clock_from_the_last_input() {
    let idle = SimIdle::new(Arc::new(SimClock::new(tokio::time::Instant::now())));
    tokio::time::advance(std::time::Duration::from_secs(600)).await;
    assert_eq!(idle.idle_seconds(), Some(0), "a node starts active");
    idle.set_idle();
    tokio::time::advance(std::time::Duration::from_secs(301)).await;
    assert_eq!(idle.idle_seconds(), Some(301));
    idle.set_active();
    assert_eq!(idle.idle_seconds(), Some(0));
}

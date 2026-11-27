use super::*;

#[tokio::test]
async fn the_first_reason_decides_the_exit() {
    let shutdown = Shutdown::new();
    assert!(!shutdown.is_requested());
    shutdown.request(ExitReason::HandlerPanic);
    shutdown.request(ExitReason::Requested);
    shutdown.requested().await;
    assert_eq!(shutdown.reason(), ExitReason::HandlerPanic);
    assert_eq!(shutdown.reason().exit_code(), 70);
}

#[tokio::test]
async fn run_until_drops_work_at_shutdown() {
    let shutdown = Shutdown::new();
    assert_eq!(shutdown.run_until(async { 7 }).await, Some(7));
    shutdown.request(ExitReason::Requested);
    assert_eq!(shutdown.run_until(std::future::pending::<()>()).await, None);
}

#[test]
fn no_request_exits_cleanly() {
    assert_eq!(Shutdown::new().reason().exit_code(), 0);
}

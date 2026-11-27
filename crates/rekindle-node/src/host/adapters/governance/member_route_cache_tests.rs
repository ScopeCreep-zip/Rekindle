use super::super::lifecycle::RouteCallError;

/// Plan E3.4 S4, lead decision: Veilid 0.5.8 loses a call into a
/// released route as a timeout, so a timeout re-reads `M`/6 as
/// `RouteUnusable` does; any other failure keeps the route.
#[test]
fn a_timed_out_or_unusable_call_rereads_the_route() {
    assert!(RouteCallError::Unusable.rereads_route());
    assert!(RouteCallError::TimedOut("Timeout".into()).rereads_route());
    assert!(!RouteCallError::Failed("not attached".into()).rereads_route());
}

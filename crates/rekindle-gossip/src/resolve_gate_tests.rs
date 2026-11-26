use super::ResolveGate;

#[tokio::test]
async fn second_leader_blocked_until_first_drops() {
    let gate = ResolveGate::new();
    let guard = gate.try_lead("c", "p").expect("first lead");
    assert!(gate.try_lead("c", "p").is_none(), "second lead must fail");
    // Different peer is independent.
    assert!(gate.try_lead("c", "q").is_some());
    drop(guard);
    assert!(gate.try_lead("c", "p").is_some(), "lead free after drop");
}

#[tokio::test]
async fn wait_returns_after_leader_finishes() {
    let gate = std::sync::Arc::new(ResolveGate::new());
    let guard = gate.try_lead("c", "p").expect("lead");
    let waiter = {
        let gate = std::sync::Arc::clone(&gate);
        tokio::spawn(async move { gate.wait("c", "p").await })
    };
    tokio::task::yield_now().await;
    assert!(!waiter.is_finished(), "waiter must block while led");
    drop(guard);
    waiter.await.expect("waiter completes");
}

//! The DirectEval registration guard (`PendingEval`).

use std::sync::Arc;

use super::EvalResultRegistry;

fn pending(registry: &EvalResultRegistry) -> usize {
    registry.pending.lock().len()
}

#[test]
fn a_dropped_waiter_releases_its_registration() {
    let registry = Arc::new(EvalResultRegistry::default());
    let (guard, rx) = registry.register_pending("wdio_de_1".to_owned());
    assert_eq!(pending(&registry), 1);
    drop(rx);
    drop(guard);
    assert_eq!(
        pending(&registry),
        0,
        "an unsettled registration must not outlive its waiter"
    );
}

#[tokio::test]
async fn a_completed_eval_is_delivered_and_its_guard_is_a_no_op() {
    let registry = Arc::new(EvalResultRegistry::default());
    let (guard, rx) = registry.register_pending("wdio_de_2".to_owned());
    registry.complete("wdio_de_2", serde_json::json!(5));
    assert_eq!(rx.await.expect("delivered"), serde_json::json!(5));
    drop(guard);
    assert_eq!(pending(&registry), 0);
}

#[tokio::test]
async fn a_waiter_dropped_by_an_outer_timeout_releases_its_registration() {
    let registry = Arc::new(EvalResultRegistry::default());
    let waiter = {
        let registry = Arc::clone(&registry);
        async move {
            let (_registration, rx) = registry.register_pending("wdio_de_3".to_owned());
            rx.await
        }
    };
    let outcome = tokio::time::timeout(std::time::Duration::from_millis(20), waiter).await;
    assert!(outcome.is_err(), "nothing posts, so the outer timer fires");
    assert_eq!(
        pending(&registry),
        0,
        "the dropped waiter released its registration"
    );
}

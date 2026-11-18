//! A row whose send another write carried out leaves its stream `sent`.

use super::*;

/// A governance-op stream held at a row that can never land (aimed at a
/// member record whose owner key is gone): once the host wrote its op
/// elsewhere and settles it landed, it is `sent` and the row behind it is
/// attempted; a row already settled, or unknown, is left alone.
#[tokio::test]
async fn a_row_settled_landed_frees_its_stream() {
    let o = outbox();
    let gov = StreamKind::GovernanceOp;
    script(&o, [AttemptOutcome::Deferred]);
    o.enqueue(entry(gov, "community", "stale"), ())
        .await
        .unwrap();
    o.enqueue(entry(gov, "community", "next"), ())
        .await
        .unwrap();
    o.pump().await.unwrap();
    assert_eq!(o.deps().store.lock().attempted, ["stale"]);
    let settled = o
        .settle_landed(&["stale".into(), "unknown".into()])
        .await
        .unwrap();
    assert_eq!(settled.len(), 1);
    assert_eq!(
        o.deps().store.lock().published.last(),
        Some(&("stale".into(), SendState::Sent))
    );
    assert!(o.settle_landed(&["stale".into()]).await.unwrap().is_empty());
    o.pump().await.unwrap();
    assert_eq!(o.deps().store.lock().attempted, ["stale", "next"]);
}

//! The `media_key` stream (plan E2.5, C7.22): retried until the recipient
//! acknowledges, settled unsent once superseded, discarded with its
//! session, and never the user's to retry.

use super::*;

#[tokio::test]
async fn an_offline_recipient_is_retried_until_its_ack() {
    let o = outbox();
    script(&o, [AttemptOutcome::Deferred, AttemptOutcome::Receipted]);
    o.enqueue(entry(StreamKind::MediaKey, "c/ch/bob", "k1"), ())
        .await
        .unwrap();
    o.pump().await.unwrap();
    assert_eq!(state_of(&o, "k1"), Some(SendState::Queued));
    advance(&o, 60_000);
    o.pump().await.unwrap();
    assert_eq!(state_of(&o, "k1"), None);
    assert_eq!(
        o.deps().store.lock().published.last(),
        Some(&("k1".into(), SendState::Delivered))
    );
}

#[tokio::test]
async fn a_superseded_key_settles_unsent_and_frees_its_stream() {
    let o = outbox();
    script(&o, [AttemptOutcome::Superseded("superseded".into())]);
    for id in ["old", "new"] {
        o.enqueue(entry(StreamKind::MediaKey, "c/ch/bob", id), ())
            .await
            .unwrap();
    }
    o.pump().await.unwrap();
    o.pump().await.unwrap();
    assert_eq!(state_of(&o, "old"), None);
    assert_eq!(o.deps().store.lock().attempted, ["old", "new"]);
}

#[tokio::test]
async fn a_session_end_discards_only_that_sessions_rows() {
    let o = outbox();
    for (scope, id) in [("c/ch/bob", "a"), ("c/ch/carol", "b"), ("c/ch2/bob", "c")] {
        o.enqueue(entry(StreamKind::MediaKey, scope, id), ())
            .await
            .unwrap();
    }
    let gone = (o.discard_streams(StreamKind::MediaKey, "c/ch/", "session ended"))
        .await
        .unwrap();
    assert_eq!(gone, 2);
    assert_eq!(state_of(&o, "a"), None);
    assert_eq!(state_of(&o, "b"), None);
    assert_eq!(state_of(&o, "c"), Some(SendState::Queued));
}

#[tokio::test]
async fn retry_refuses_a_media_key_row() {
    let o = outbox();
    script(&o, std::iter::repeat_n(AttemptOutcome::Deferred, 64));
    o.enqueue(entry(StreamKind::MediaKey, "c/ch/bob", "k"), ())
        .await
        .unwrap();
    let deadline = o.deps().store.lock().rows[0].deadline;
    assert!(matches!(
        o.retry("k").await,
        Err(OutboxError::NotRetryable(StreamKind::MediaKey))
    ));
    assert_eq!(o.deps().store.lock().rows[0].deadline, deadline);
}

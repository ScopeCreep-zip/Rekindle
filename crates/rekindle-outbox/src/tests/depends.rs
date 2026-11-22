//! Plan HOL1 (lead decision 47): a peer's `friend_handshake` rows wait only
//! for the row they name, never for their place in the stream; a failed
//! row fails the rows waiting for it, and the user's Retry queues them
//! again with it (Signal-Android `JobController.onFailure`).

use super::*;

fn fh() -> StreamKind {
    StreamKind::FriendHandshake
}

async fn waiting(o: &Outbox<Fake>, id: &str, waits_for: &[&str]) {
    let waits = waits_for.iter().map(|p| (*p).to_owned()).collect();
    o.enqueue_waiting(entry(fh(), "bob", id), waits, ())
        .await
        .unwrap();
}

fn attempted(o: &Outbox<Fake>) -> Vec<String> {
    o.deps().store.lock().attempted.clone()
}

/// A presence key waits for our unsettled first contact, named; it goes
/// once that row settles, and not before.
#[tokio::test]
async fn a_row_gates_only_on_its_named_dependency() {
    let o = outbox();
    script(&o, [AttemptOutcome::Unanswered]);
    waiting(&o, "FriendAccept:1", &[]).await;
    waiting(&o, "FriendsKey:1", &["FriendAccept:"]).await;
    let row = o.deps().row("FriendsKey:1").await.unwrap().unwrap();
    assert_eq!(row.depends_on.as_deref(), Some("FriendAccept:1"));
    o.pump().await.unwrap();
    o.pump().await.unwrap();
    assert_eq!(attempted(&o), ["FriendAccept:1"]);
    assert_eq!(state_of(&o, "FriendsKey:1"), Some(SendState::Queued));
    // The peer's frame under the session settles the first contact.
    let stream = fh().stream("bob");
    assert_eq!(o.receipt_rows(&stream, "FriendAccept:").await.unwrap(), 1);
    o.pump().await.unwrap();
    assert_eq!(attempted(&o), ["FriendAccept:1", "FriendsKey:1"]);
}

/// A row that names no dependency is attempted past an earlier row of the
/// same stream that stays behind (deferred), in the same run.
#[tokio::test]
async fn an_independent_row_proceeds_past_a_deferred_one() {
    let o = outbox();
    script(&o, [AttemptOutcome::Deferred]);
    waiting(&o, "FriendsKey:0", &[]).await;
    waiting(&o, "GroupDmInvite:0", &[]).await;
    o.pump().await.unwrap();
    assert_eq!(attempted(&o), ["FriendsKey:0", "GroupDmInvite:0"]);
    assert_eq!(state_of(&o, "FriendsKey:0"), Some(SendState::Queued));
    assert_eq!(state_of(&o, "GroupDmInvite:0"), None, "settled sent");
}

/// The first contact wedges: the presence key waiting for it fails with
/// it (an independent row does not), and Retry of the first contact
/// queues both again, the key going once the first contact settled.
#[tokio::test]
async fn a_failed_dependency_fails_its_dependents_and_retry_requeues_them() {
    let o = outbox();
    script(&o, [AttemptOutcome::Wedged("prekey_rejected".into())]);
    waiting(&o, "FriendAccept:1", &[]).await;
    waiting(&o, "FriendsKey:1", &["FriendAccept:"]).await;
    waiting(&o, "GroupDmInvite:1", &[]).await;
    o.pump().await.unwrap();
    assert_eq!(state_of(&o, "FriendAccept:1"), Some(SendState::Failed));
    let key = o.deps().row("FriendsKey:1").await.unwrap().unwrap();
    assert_eq!(
        (key.state, key.wedge_reason.as_deref()),
        (SendState::Failed, Some("the frame it waits for failed"))
    );
    assert_eq!(state_of(&o, "GroupDmInvite:1"), None, "settled sent");
    assert!(o.retry("FriendAccept:1").await.unwrap());
    assert_eq!(state_of(&o, "FriendAccept:1"), Some(SendState::Queued));
    assert_eq!(state_of(&o, "FriendsKey:1"), Some(SendState::Queued));
    o.pump().await.unwrap();
    o.pump().await.unwrap();
    assert_eq!(state_of(&o, "FriendAccept:1"), None);
    assert_eq!(state_of(&o, "FriendsKey:1"), None);
    let calls = attempted(&o);
    assert_eq!(
        &calls[calls.len() - 2..],
        ["FriendAccept:1", "FriendsKey:1"]
    );
}

/// Finding 70 refined: a request cancelled right after it was queued and
/// attempted (its first send wedged it) leaves no handshake row.
#[tokio::test]
async fn a_request_cancelled_after_its_first_attempt_leaves_no_row() {
    let o = outbox();
    script(&o, [AttemptOutcome::Wedged("no bundle".into())]);
    let fh = StreamKind::FriendHandshake;
    o.enqueue(entry(fh, "bob", "req"), ()).await.unwrap();
    o.pump().await.unwrap();
    assert_eq!(state_of(&o, "req"), Some(SendState::Failed));
    assert_eq!(o.discard_streams(fh, "bob", "cancelled").await.unwrap(), 1);
    assert_eq!(state_of(&o, "req"), None);
}

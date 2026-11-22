//! The engine's core lifecycle: the state lattice, Sent vs Delivered, read
//! markers, the stream head, deadlines, Retry and Discard, durable rows and
//! the jittered backoff.

use super::*;

#[test]
fn the_lattice_only_climbs_and_failed_only_replaces_unsettled() {
    use SendState::*;
    assert_eq!(merge(Queued, StateInput::LocalSent), Sent);
    assert_eq!(merge(Delivered, StateInput::LocalSent), Delivered);
    assert_eq!(merge(Read, StateInput::Peer(PeerAck::Delivered)), Read);
    assert_eq!(merge(Sent, StateInput::Peer(PeerAck::Read)), Read);
    assert_eq!(merge(Sent, StateInput::Failed), Failed);
    assert_eq!(merge(Queued, StateInput::Failed), Failed);
    assert_eq!(merge(Delivered, StateInput::Failed), Delivered);
    assert_eq!(merge(Failed, StateInput::LocalSent), Failed);
    assert_eq!(
        merge(Failed, StateInput::Peer(PeerAck::Delivered)),
        Delivered
    );
}

#[tokio::test]
async fn a_local_ok_is_sent_and_only_the_peer_makes_it_delivered() {
    let o = outbox();
    let at = o
        .enqueue(entry(StreamKind::Dm, "bob", "a"), ())
        .await
        .unwrap();
    o.pump().await.unwrap();
    assert_eq!(state_of(&o, "a"), Some(SendState::Sent));
    // Unacknowledged, it is sent again later under the same id.
    advance(&o, 600_000);
    o.pump().await.unwrap();
    assert_eq!(o.deps().store.lock().attempted, ["a", "a"]);
    assert!(o
        .acknowledge(&at.stream, at.stream_seq, PeerAck::Delivered)
        .await
        .unwrap());
    assert_eq!(state_of(&o, "a"), None);
    let published = o.deps().store.lock().published.clone();
    let states: Vec<_> = published.into_iter().map(|(_, s)| s).collect();
    assert_eq!(
        states,
        [SendState::Queued, SendState::Sent, SendState::Delivered]
    );
    assert!(!o
        .acknowledge(&at.stream, at.stream_seq, PeerAck::Read)
        .await
        .unwrap());
}

#[tokio::test]
async fn read_up_to_raises_every_entry_at_or_below_it_once() {
    let o = outbox();
    let a = o
        .enqueue(entry(StreamKind::Dm, "bob", "a"), ())
        .await
        .unwrap();
    o.enqueue(entry(StreamKind::Dm, "bob", "b"), ())
        .await
        .unwrap();
    o.enqueue(entry(StreamKind::Dm, "bob", "c"), ())
        .await
        .unwrap();
    o.enqueue(entry(StreamKind::Dm, "carol", "d"), ())
        .await
        .unwrap();
    o.pump().await.unwrap();
    // `a` was delivered and settled; `b` waits behind it, not yet landed,
    // so the read shows it read at once (D22) and keeps it to its landing
    // (live A/B finding 42 follow-up); `c` is past the marker.
    o.acknowledge(&a.stream, a.stream_seq, PeerAck::Delivered)
        .await
        .unwrap();
    o.deps().store.lock().published.clear();
    assert_eq!(o.read_up_to(StreamKind::Dm, "bob", 2).await.unwrap(), 2);
    let mut read = o.deps().store.lock().published.clone();
    read.sort_by(|x, y| x.0.cmp(&y.0));
    assert_eq!(
        read,
        [("a".into(), SendState::Read), ("b".into(), SendState::Read)]
    );
    assert_eq!(state_of(&o, "b"), Some(SendState::Queued), "marked, kept");
    let unread = |id| state_of(&o, id).is_some_and(|s| s != SendState::Read);
    assert!(unread("c"), "past the marker");
    assert!(unread("d"), "another stream");
    assert_eq!(o.read_up_to(StreamKind::Dm, "bob", 2).await.unwrap(), 0);
}

#[tokio::test]
async fn a_dht_write_settles_when_sent() {
    let o = outbox();
    o.enqueue(entry(StreamKind::DhtWrite, "rec/0", "w"), ())
        .await
        .unwrap();
    assert_eq!(o.unsettled().await.unwrap(), 1);
    o.pump().await.unwrap();
    assert_eq!(o.unsettled().await.unwrap(), 0);
}

#[tokio::test]
async fn the_stream_head_blocks_its_stream_only() {
    let o = outbox();
    script(&o, [AttemptOutcome::Deferred]);
    o.enqueue(entry(StreamKind::Dm, "bob", "a"), ())
        .await
        .unwrap();
    o.enqueue(entry(StreamKind::Dm, "bob", "b"), ())
        .await
        .unwrap();
    o.enqueue(entry(StreamKind::Dm, "carol", "c"), ())
        .await
        .unwrap();
    let next = o.pump().await.unwrap().unwrap();
    assert_eq!(o.deps().store.lock().attempted, ["a", "c"]);
    assert_eq!(state_of(&o, "b"), Some(SendState::Queued));
    advance(&o, next - 1_000);
    o.pump().await.unwrap();
    o.pump().await.unwrap();
    assert_eq!(o.deps().store.lock().attempted, ["a", "c", "a", "b"]);
}

#[tokio::test]
async fn past_the_deadline_a_row_fails_and_retry_keeps_its_id() {
    let o = outbox();
    script(&o, std::iter::repeat_n(AttemptOutcome::Deferred, 64));
    o.enqueue(entry(StreamKind::Dm, "bob", "a"), ())
        .await
        .unwrap();
    o.enqueue(entry(StreamKind::Dm, "bob", "b"), ())
        .await
        .unwrap();
    o.pump().await.unwrap();
    advance(
        &o,
        i64::try_from(rekindle_crypto::horizon::DELIVERY_HORIZON.as_millis()).unwrap(),
    );
    o.pump().await.unwrap();
    let row = o.deps().row("a").await.unwrap().unwrap();
    assert_eq!(
        (row.state, row.wedge_reason.as_deref()),
        (SendState::Failed, Some(DEADLINE_REASON))
    );
    // `b` passed the same deadline; a local send never revives a failed row.
    assert_eq!(state_of(&o, "b"), Some(SendState::Failed));
    o.deps().store.lock().outcomes.clear();
    assert!(o.retry("a").await.unwrap());
    assert_eq!(state_of(&o, "a"), Some(SendState::Queued));
    o.pump().await.unwrap();
    assert_eq!(
        o.deps().store.lock().attempted.last().map(String::as_str),
        Some("a")
    );
    assert_eq!(state_of(&o, "a"), Some(SendState::Sent));
}

#[tokio::test]
async fn discard_settles_failed_and_frees_the_stream() {
    let o = outbox();
    script(&o, [AttemptOutcome::Wedged("prekey_rejected".into())]);
    o.enqueue(entry(StreamKind::Dm, "bob", "a"), ())
        .await
        .unwrap();
    o.enqueue(entry(StreamKind::Dm, "bob", "b"), ())
        .await
        .unwrap();
    o.pump().await.unwrap();
    o.pump().await.unwrap();
    assert_eq!(o.deps().store.lock().attempted, ["a"]);
    assert!(o.discard("a").await.unwrap());
    assert_eq!(state_of(&o, "a"), None);
    assert_eq!(
        o.deps().store.lock().published.last(),
        Some(&("a".into(), SendState::Failed))
    );
    o.pump().await.unwrap();
    assert_eq!(o.deps().store.lock().attempted, ["a", "b"]);
}

#[tokio::test]
async fn rows_outlive_the_engine_and_resends_are_received_once() {
    let first = outbox();
    script(&first, [AttemptOutcome::Deferred]);
    first
        .enqueue(entry(StreamKind::Dm, "bob", "a"), ())
        .await
        .unwrap();
    first.pump().await.unwrap();
    // A restart: a new engine over the same rows sends the same id.
    let again = Outbox::new(first.deps().clone());
    advance(&again, 20_000);
    again.pump().await.unwrap();
    assert_eq!(again.deps().store.lock().attempted, ["a", "a"]);
    assert!(again.receive("dm:bob", "a").await.unwrap());
    assert!(!again.receive("dm:bob", "a").await.unwrap());
}

#[test]
fn backoff_is_jittered_within_its_step_and_capped() {
    let rng = CounterRng(AtomicU64::new(3));
    let ms = |kind: StreamKind, n| kind.policy().delay(n, &rng).as_millis();
    let first = ms(StreamKind::GroupDm, 1);
    assert!((500..=1_000).contains(&first), "{first}");
    let third = ms(StreamKind::GroupDm, 3);
    assert!((2_000..=4_000).contains(&third), "{third}");
    let late = ms(StreamKind::MediaKey, 30);
    assert!((7_500..=15_000).contains(&late), "{late}");
    let again = CounterRng(AtomicU64::new(3));
    assert_eq!(
        StreamKind::GroupDm.policy().delay(1, &again).as_millis(),
        first
    );
    assert_eq!(
        StreamKind::parse("friend_handshake"),
        Some(StreamKind::FriendHandshake)
    );
    assert_eq!(StreamKind::parse("group_dm"), Some(StreamKind::GroupDm));
    assert_eq!(
        StreamKind::ChannelPost.policy(),
        StreamKind::GovernanceOp.policy()
    );
}

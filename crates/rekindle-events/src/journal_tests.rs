use super::*;
use rekindle_types::subscription_events::NetworkEvent;

fn ev(key: &str) -> SubscriptionEvent {
    SubscriptionEvent::Network(NetworkEvent::ValueChanged {
        record_key: key.into(),
        changed_subkeys: vec![],
    })
}

fn seq_of(frame: &StreamFrame) -> Option<u64> {
    match frame {
        StreamFrame::Event(s) => Some(s.cursor.seq),
        _ => None,
    }
}

#[tokio::test]
async fn a_channel_stream_yields_its_sender_s_frames_then_ends() {
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    let mut stream = EventStream::from_channel(rx);
    tx.send(StreamFrame::InvalidSession).await.unwrap();
    drop(tx);
    assert!(matches!(
        stream.next().await,
        Some(StreamFrame::InvalidSession)
    ));
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn a_resumed_subscriber_gets_what_it_missed_then_live_with_no_gap() {
    let journal = SessionJournal::new(16, 16, Arc::new(rekindle_utils::random::OsRandom));
    let first = journal.publish(ev("a"));
    journal.publish(ev("b"));
    journal.publish(ev("c"));
    let mut stream = journal.subscribe(Some(first));
    journal.publish(ev("d"));
    assert_eq!(seq_of(&stream.next().await.unwrap()), Some(2));
    assert_eq!(seq_of(&stream.next().await.unwrap()), Some(3));
    assert!(matches!(
        stream.next().await.unwrap(),
        StreamFrame::Resumed(Cursor { seq: 3, .. })
    ));
    assert_eq!(seq_of(&stream.next().await.unwrap()), Some(4));
}

#[tokio::test]
async fn an_evicted_or_foreign_cursor_is_an_invalid_session() {
    let journal = SessionJournal::new(2, 16, Arc::new(rekindle_utils::random::OsRandom));
    let gone = journal.publish(ev("a"));
    journal.publish(ev("b"));
    journal.publish(ev("c"));
    journal.publish(ev("d"));
    assert!(matches!(
        journal.subscribe(Some(gone)).next().await,
        Some(StreamFrame::InvalidSession)
    ));
    let foreign = Cursor {
        epoch: journal.head().epoch ^ 1,
        seq: 1,
    };
    assert!(matches!(
        journal.subscribe(Some(foreign)).next().await,
        Some(StreamFrame::InvalidSession)
    ));
}

#[tokio::test]
async fn a_new_epoch_invalidates_live_subscribers_and_empties_the_ring() {
    let journal = SessionJournal::new(16, 16, Arc::new(rekindle_utils::random::OsRandom));
    let before = journal.publish(ev("a"));
    let mut live = journal.subscribe(None);
    journal.begin();
    assert!(matches!(
        live.next().await,
        Some(StreamFrame::InvalidSession)
    ));
    assert_ne!(journal.head().epoch, before.epoch);
    assert_eq!(journal.head().seq, 0);
    assert!(matches!(
        journal.subscribe(Some(before)).next().await,
        Some(StreamFrame::InvalidSession)
    ));
}

#[tokio::test]
async fn a_subscriber_that_falls_behind_reloads_and_its_stream_ends() {
    let journal = SessionJournal::new(64, 2, Arc::new(rekindle_utils::random::OsRandom));
    let mut live = journal.subscribe(None);
    for i in 0..8 {
        journal.publish(ev(&i.to_string()));
    }
    assert!(matches!(
        live.next().await,
        Some(StreamFrame::InvalidSession)
    ));
    assert!(live.next().await.is_none());
}

#[tokio::test]
async fn a_standing_event_raised_before_anyone_connected_reaches_a_later_connection() {
    let journal = SessionJournal::new(16, 16, Arc::new(rekindle_utils::random::OsRandom));
    journal.begin();
    journal.publish(ev("news"));
    let alert = journal.publish_standing(ev("alert"));
    journal.publish(ev("more news"));
    let mut late = journal.connect(None);
    journal.publish(ev("live"));
    let first = late.next().await.unwrap();
    assert!(
        matches!(&first, StreamFrame::Event(s) if s.cursor == alert),
        "the standing event comes first: {first:?}"
    );
    assert_eq!(seq_of(&late.next().await.unwrap()), Some(4));
    assert_eq!(journal.standing().len(), 1);
    // A plain live subscription stays live only.
    let mut live = journal.subscribe(None);
    journal.publish(ev("after"));
    assert_eq!(seq_of(&live.next().await.unwrap()), Some(5));
}

#[tokio::test]
async fn a_state_published_before_a_connection_is_its_snapshot_then_changes_arrive_live() {
    let journal = SessionJournal::new(16, 16, Arc::new(rekindle_utils::random::OsRandom));
    journal.begin();
    journal.publish_state("roster", ev("roster: a"));
    let current = journal.publish_state("roster", ev("roster: a, b"));
    let mut late = journal.connect(None);
    let later = journal.publish_state("roster", ev("roster: a, b, c"));
    let first = late.next().await.unwrap();
    assert!(
        matches!(&first, StreamFrame::Event(s) if s.cursor == current),
        "the snapshot is the state as it is now, not its history: {first:?}"
    );
    let next = late.next().await.unwrap();
    assert!(
        matches!(&next, StreamFrame::Event(s) if s.cursor == later),
        "a later change arrives live: {next:?}"
    );
    assert_eq!(journal.standing().len(), 1);
    journal.clear_state("roster");
    assert!(journal.standing().is_empty());
}

#[tokio::test]
async fn a_new_session_drops_the_standing_events_and_a_resync_keeps_them() {
    let journal = SessionJournal::new(16, 16, Arc::new(rekindle_utils::random::OsRandom));
    journal.publish_standing(ev("alert"));
    journal.publish(ev("news"));
    journal.resync();
    let kept = journal.standing();
    assert_eq!(kept.len(), 1);
    assert_eq!(
        kept[0].cursor,
        journal.head(),
        "re-sequenced in the new epoch"
    );
    assert!(matches!(
        journal.connect(None).next().await,
        Some(StreamFrame::Event(s)) if s.cursor == kept[0].cursor
    ));
    journal.begin();
    assert!(journal.standing().is_empty());
}

#[tokio::test]
async fn a_frontend_started_again_with_its_cursor_is_shown_the_standing_events_again() {
    let journal = SessionJournal::new(16, 16, Arc::new(rekindle_utils::random::OsRandom));
    let alert = journal.publish_standing(ev("alert"));
    let kept = journal.publish(ev("news"));
    journal.publish(ev("missed"));
    let mut resumed = journal.connect(Some(kept));
    assert!(matches!(
        resumed.next().await,
        Some(StreamFrame::Event(s)) if s.cursor == alert
    ));
    assert_eq!(seq_of(&resumed.next().await.unwrap()), Some(3));
    assert!(matches!(
        resumed.next().await,
        Some(StreamFrame::Resumed(Cursor { seq: 3, .. }))
    ));
    // A cursor the epoch cannot serve gets only `InvalidSession`.
    let foreign = Cursor {
        epoch: kept.epoch ^ 1,
        seq: 1,
    };
    let mut invalid = journal.connect(Some(foreign));
    assert!(matches!(
        invalid.next().await,
        Some(StreamFrame::InvalidSession)
    ));
}

#[test]
fn epochs_are_exact_in_javascript() {
    for _ in 0..64 {
        assert!(new_epoch(&rekindle_utils::random::OsRandom) < (1 << 53));
    }
}

#[tokio::test]
async fn the_stop_frame_reaches_live_streams_once_and_is_never_replayed() {
    let journal = SessionJournal::new(16, 16, Arc::new(rekindle_utils::random::OsRandom));
    journal.publish(ev("a"));
    let mut live = journal.subscribe(None);
    journal.close(StopReason::DataWiped);
    journal.close(StopReason::Requested);
    journal.publish(ev("b"));
    assert!(matches!(
        live.next().await,
        Some(StreamFrame::HostStopping {
            reason: StopReason::DataWiped
        })
    ));
    assert_eq!(live.next().await.as_ref().and_then(seq_of), Some(2));
    let replay = journal.replay(Cursor {
        epoch: journal.head().epoch,
        seq: 0,
    });
    assert!(replay
        .iter()
        .all(|f| !matches!(f, StreamFrame::HostStopping { .. })));
}

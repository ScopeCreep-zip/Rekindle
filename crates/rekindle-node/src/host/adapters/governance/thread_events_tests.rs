//! `thread_creations`: a thread the merge adds is announced once, from the
//! merged state alone (plan E3.2 S8-E), and a re-merge announces nothing.

use rekindle_governance::fixture::stamp;
use rekindle_governance::state::{GovernanceState, ThreadState};
use rekindle_types::id::{ChannelId, PseudonymKey, ThreadId};
use rekindle_types::subscription_events::SocialEvent;

use super::thread_creations;

const C: &str = "community";

fn thread() -> ThreadState {
    ThreadState {
        parent_channel_id: ChannelId([7; 16]),
        name: "design".into(),
        thread_type: "public".into(),
        invited: Vec::new(),
        forum_tag: Some("question".into()),
        auto_archive_seconds: 3_600,
        creator: PseudonymKey([2; 32]),
        created: stamp(4),
        archived: None,
        unarchived: None,
    }
}

#[test]
fn a_merged_thread_create_is_announced_once() {
    let prev = GovernanceState::default();
    let mut next = prev.clone();
    next.threads.insert(ThreadId([9; 16]), thread());

    let events = thread_creations(C, &prev, &next, 1_000);
    assert_eq!(events.len(), 1);
    let SocialEvent::ThreadCreated { community, thread } = &events[0] else {
        panic!("expected ThreadCreated, got {:?}", events[0]);
    };
    assert_eq!(community, C);
    assert_eq!(thread.id, hex::encode([9; 16]));
    assert_eq!(thread.channel_id, hex::encode([7; 16]));
    assert_eq!(thread.name, "design");
    assert_eq!(thread.creator_pseudonym, hex::encode([2; 32]));
    assert_eq!(thread.forum_tag.as_deref(), Some("question"));
    assert_eq!(thread.auto_archive_seconds, 3_600);
    assert_eq!(thread.created_at, 1_000);
    assert!(!thread.archived);

    // The same state merged again adds no thread.
    assert!(thread_creations(C, &next, &next, 1_001).is_empty());
}

#[test]
fn a_thread_already_known_is_not_announced() {
    let mut prev = GovernanceState::default();
    prev.threads.insert(ThreadId([9; 16]), thread());
    let mut next = prev.clone();
    next.threads
        .get_mut(&ThreadId([9; 16]))
        .expect("thread")
        .archived = Some(stamp(5));
    assert!(thread_creations(C, &prev, &next, 1_000).is_empty());
}

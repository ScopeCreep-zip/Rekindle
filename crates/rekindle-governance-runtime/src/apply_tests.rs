use super::*;
use rekindle_types::id::ChannelId;

#[test]
fn classify_role_definition_marks_roles() {
    let entry = GovernanceEntry::RoleDefinition {
        role_id: rekindle_types::id::RoleId([0u8; 16]),
        name: "test".into(),
        permissions: 0,
        position: 0,
        color: 0,
        hoist: false,
        mentionable: false,
        self_assignable: false,
        exclusion_group: None,
    };
    let affects = classify_entry(&entry);
    assert!(affects.roles);
    assert!(!affects.channels_or_categories);
}

#[test]
fn classify_role_archived_marks_roles() {
    let entry = GovernanceEntry::RoleArchived {
        role_id: rekindle_types::id::RoleId([0u8; 16]),
    };
    let affects = classify_entry(&entry);
    assert!(affects.roles);
    assert!(!affects.channels_or_categories);
}

#[test]
fn classify_channel_created_marks_channels() {
    let entry = GovernanceEntry::ChannelCreated {
        channel_id: ChannelId([0u8; 16]),
        name: "general".into(),
        channel_type: "text".into(),
        category_id: None,
        position: 0,
        parent_voice_channel_id: None,
    };
    let affects = classify_entry(&entry);
    assert!(!affects.roles);
    assert!(affects.channels_or_categories);
}

#[test]
fn classify_category_updated_marks_channels() {
    let entry = GovernanceEntry::CategoryUpdated {
        category_id: rekindle_types::id::CategoryId([0u8; 16]),
        name: Some("renamed".into()),
        position: None,
    };
    let affects = classify_entry(&entry);
    assert!(!affects.roles);
    assert!(affects.channels_or_categories);
}

#[test]
fn out_of_bounds_entries_are_refused_by_field() {
    use rekindle_types::channel::MAX_SLOWMODE_SECONDS;
    use rekindle_types::key_format::MAX_DESCRIPTION_LEN;
    use rekindle_types::member::MAX_TIMEOUT_SECONDS;
    let slowmode = |seconds| GovernanceEntry::ChannelUpdated {
        channel_id: ChannelId([0u8; 16]),
        name: None,
        topic: None,
        forum_tags: None,
        position: None,
        slowmode_seconds: Some(seconds),
        nsfw: None,
        category_id: None,
    };
    let timeout = |duration_seconds| GovernanceEntry::TimeoutEntry {
        target: PseudonymKey([0u8; 32]),
        duration_seconds,
        reason: None,
        started_at: 0,
    };
    let meta = |description: String| GovernanceEntry::CommunityMeta {
        name: None,
        description: Some(description),
        icon_hash: None,
        banner_hash: None,
    };
    let field = |entry: &GovernanceEntry| match refuse_out_of_bounds(entry) {
        Err(GovernanceRuntimeError::OutOfBounds { field }) => Some(field),
        Err(other) => panic!("unexpected {other}"),
        Ok(()) => None,
    };
    assert_eq!(
        field(&slowmode(MAX_SLOWMODE_SECONDS + 1)),
        Some("slowmode_seconds")
    );
    assert_eq!(field(&slowmode(MAX_SLOWMODE_SECONDS)), None);
    assert_eq!(field(&timeout(0)), Some("duration_seconds"));
    assert_eq!(
        field(&timeout(MAX_TIMEOUT_SECONDS + 1)),
        Some("duration_seconds")
    );
    assert_eq!(field(&timeout(MAX_TIMEOUT_SECONDS)), None);
    let long = "x".repeat(MAX_DESCRIPTION_LEN + 1);
    assert_eq!(field(&meta(long)), Some("description"));
    assert_eq!(field(&meta("ok\n".into())), None);
}

/// `write_entry` refuses before touching anything: the mock has no
/// expectations, so any state lookup or DHT call would panic.
#[tokio::test]
async fn write_entry_returns_out_of_bounds_before_any_io() {
    let deps = crate::deps::MockGovernanceRuntimeDeps::new();
    let entry = GovernanceEntry::TimeoutEntry {
        target: PseudonymKey([0u8; 32]),
        duration_seconds: 0,
        reason: None,
        started_at: 0,
    };
    let refused = write_entry(&deps, "community", entry).await;
    assert!(matches!(
        refused,
        Err(GovernanceRuntimeError::OutOfBounds {
            field: "duration_seconds"
        })
    ));
}

/// A write reads the state and op set only under the rebuild lock
/// (register S7l, HIGH-2): while a rebuild holds it, the write reads
/// nothing, so a rebuild that read an older op set cannot install over
/// the write's state.
#[tokio::test]
async fn a_write_waits_for_a_running_rebuild_before_reading_the_state() {
    let held = std::sync::Arc::new(tokio::sync::Mutex::new(()));
    let read = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut deps = crate::deps::MockGovernanceRuntimeDeps::new();
    let lock = std::sync::Arc::clone(&held);
    deps.expect_rebuild_lock()
        .returning(move |_| std::sync::Arc::clone(&lock));
    let seen = std::sync::Arc::clone(&read);
    deps.expect_governance_state().returning(move |_| {
        seen.store(true, std::sync::atomic::Ordering::SeqCst);
        None
    });
    let deps = std::sync::Arc::new(deps);
    let rebuilding = held.lock().await;
    let writer = std::sync::Arc::clone(&deps);
    let write = tokio::spawn(async move {
        write_entry(
            writer.as_ref(),
            "community",
            GovernanceEntry::MemberLeft { heads: Vec::new() },
        )
        .await
    });
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    assert!(
        !read.load(std::sync::atomic::Ordering::SeqCst),
        "the write read the state while a rebuild held the lock"
    );
    drop(rebuilding);
    let written = write.await.unwrap();
    assert!(read.load(std::sync::atomic::Ordering::SeqCst));
    assert!(matches!(
        written,
        Err(GovernanceRuntimeError::GovernanceStateMissing(_))
    ));
}

#[test]
fn classify_role_assignment_affects_nothing() {
    let entry = GovernanceEntry::RoleAssignment {
        target: PseudonymKey([0u8; 32]),
        role_id: rekindle_types::id::RoleId([0u8; 16]),
    };
    let affects = classify_entry(&entry);
    assert!(!affects.roles);
    assert!(!affects.channels_or_categories);
}

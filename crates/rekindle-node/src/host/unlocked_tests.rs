use rekindle_types::subscription_events::{NotificationEvent, SubscriptionEvent};

use super::memory_notice;

#[test]
fn every_secret_locked_raises_no_notice() {
    assert!(memory_notice(&[true, true]).is_none());
}

#[test]
fn a_refused_lock_raises_one_system_alert() {
    let Some(SubscriptionEvent::Notification(NotificationEvent::SystemAlert {
        title,
        body,
        subject,
    })) = memory_notice(&[true, false])
    else {
        panic!("a refused lock is shown to the user");
    };
    assert_eq!(title, "Keys not in locked memory");
    assert!(body.contains("ulimit -l"), "{body}");
    assert!(subject.is_none());
    assert!(memory_notice(&[false, false]).is_some());
}

fn is_memory_notice(event: &SubscriptionEvent) -> bool {
    matches!(
        event,
        SubscriptionEvent::Notification(NotificationEvent::SystemAlert { title, .. })
            if title == "Keys not in locked memory"
    )
}

/// The notice is raised while the session installs, before any frontend
/// has subscribed; both modes' late subscribers still get it.
#[tokio::test]
async fn the_memory_notice_reaches_a_frontend_that_connects_after_the_unlock() {
    use rekindle_types::subscription_events::StreamFrame;
    let dir = tempfile::tempdir().unwrap();
    let (host, _) = crate::host::gate::tests::host_with_identity(&dir).await;
    host.ctx.journal.begin();
    super::raise_memory_notice(&host.ctx, &[true, false]);
    // The daemon's bus connection, subscribing with no cursor.
    let mut connected = host.ctx.journal.connect(None);
    let frame = tokio::time::timeout(std::time::Duration::from_secs(5), connected.next())
        .await
        .unwrap();
    assert!(
        matches!(&frame, Some(StreamFrame::Event(s)) if is_memory_notice(&s.event)),
        "{frame:?}"
    );
    // The GUI's window that just opened (`subscribe_events`, no cursor).
    let standing = host.standing();
    assert_eq!(standing.len(), 1);
    assert!(is_memory_notice(&standing[0].event));
    // The next session starts without it.
    host.ctx.journal.begin();
    assert!(host.standing().is_empty());
}

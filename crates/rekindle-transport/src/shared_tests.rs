use super::*;

#[test]
fn attachment_state_round_trip() {
    for raw in 0..=8u8 {
        let state = AttachmentState::from_u8(raw);
        assert_eq!(state as u8, raw);
    }
    // Out of range falls back to Detached
    assert_eq!(AttachmentState::from_u8(255), AttachmentState::Detached);
}

#[test]
fn an_unchanged_attachment_resend_is_no_edge() {
    let state = SharedState::new();
    let mut ready = state.subscribe_ready();
    let mut rx = state.subscribe();
    state.set_attachment(AttachmentState::AttachedGood, true, true);
    assert!(ready.has_changed().expect("sender alive"));
    ready.borrow_and_update();
    assert!(rx.try_recv().is_ok());

    // Veilid 0.5.8's stall re-send: the same attachment again.
    state.set_attachment(AttachmentState::AttachedGood, true, true);
    assert!(!ready.has_changed().expect("sender alive"));
    assert!(rx.try_recv().is_err());

    // A state change with the same readiness notifies but wakes no
    // readiness waiter.
    state.set_attachment(AttachmentState::FullyAttached, true, true);
    assert!(!ready.has_changed().expect("sender alive"));
    assert!(rx.try_recv().is_ok());
}

#[tokio::test]
async fn shared_state_subscribe_receives_events() {
    let state = SharedState::new();
    let mut rx = state.subscribe();

    state.set_attachment(AttachmentState::FullyAttached, true, true);

    let event = rx.try_recv().expect("should receive notification");
    match event {
        TransportNotification::AttachmentChanged {
            state,
            is_attached,
            public_internet_ready,
        } => {
            assert_eq!(state, AttachmentState::FullyAttached);
            assert!(is_attached);
            assert!(public_internet_ready);
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[tokio::test]
async fn shared_state_cleans_up_dropped_subscribers() {
    let state = SharedState::new();
    let rx1 = state.subscribe();
    let _rx2 = state.subscribe();
    assert_eq!(state.subscriber_count(), 2);

    // Drop rx1
    drop(rx1);

    // Next notify cleans up the dead subscriber
    state.notify(&TransportNotification::LocalRoutesDied { count: 1 });
    assert_eq!(state.subscriber_count(), 1);
}

#[test]
fn shared_state_uptime_increases() {
    let state = SharedState::new();
    let t1 = state.uptime();
    std::thread::sleep(std::time::Duration::from_millis(10));
    let t2 = state.uptime();
    assert!(t2 > t1);
}

#[test]
fn shared_state_atomic_reads_match_writes() {
    let state = SharedState::new();
    assert_eq!(state.attachment_state(), AttachmentState::Detached);
    assert!(!state.is_attached());
    assert!(!state.public_internet_ready());

    state.set_attachment(AttachmentState::AttachedStrong, true, true);
    assert_eq!(state.attachment_state(), AttachmentState::AttachedStrong);
    assert!(state.is_attached());
    assert!(state.public_internet_ready());

    state.set_attachment(AttachmentState::Detaching, false, false);
    assert_eq!(state.attachment_state(), AttachmentState::Detaching);
    assert!(!state.is_attached());
    assert!(!state.public_internet_ready());
}

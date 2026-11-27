use super::*;

/// A chord another program or a second instance holds leaves the app
/// running (Linux finding: `HotKey already registered` panicked setup).
#[test]
fn already_registered_chord_is_not_fatal() {
    let refused = Err(ShortcutError::GlobalHotkey(
        "HotKey already registered: HotKey { mods: CONTROL | SHIFT, key: KeyX }".into(),
    ));
    assert!(settle_registration("ctrl+shift+x", refused).is_ok());
    assert!(settle_registration("ctrl+shift+x", Ok(())).is_ok());
}

/// A stored panic shortcut becomes the chord the plugin registers; a
/// malformed one is an error, never a quietly unarmed shortcut.
#[test]
fn the_panic_chord_parses_or_is_refused() {
    let chord = parse_panic_chord(Some("CommandOrControl+Shift+F12"))
        .unwrap()
        .unwrap();
    assert_eq!(
        chord,
        "CommandOrControl+Shift+F12".parse::<Shortcut>().unwrap()
    );
    assert!(parse_panic_chord(None).unwrap().is_none());
    assert!(parse_panic_chord(Some("Shift+NotAKey")).is_err());
}

/// The Host's `QuickWipeChanged` drives the registrar: an arming
/// registers its chord, an arming without one and a disarm release it,
/// every other event leaves it, and the follower ends on the Host's
/// stop (plan E7.1: a committed arm or disarm is live at once).
#[tokio::test]
async fn quick_wipe_changes_drive_registration_and_release() {
    use rekindle_types::subscription_events::{
        Cursor, Sequenced, StopReason, StreamFrame, SystemEvent,
    };
    let event = |seq, event| {
        StreamFrame::Event(Box::new(Sequenced {
            cursor: Cursor { epoch: 1, seq },
            event,
        }))
    };
    let changed = |armed, shortcut: Option<&str>| {
        SubscriptionEvent::System(SystemEvent::QuickWipeChanged {
            armed,
            shortcut: shortcut.map(str::to_owned),
        })
    };
    let frames = futures::stream::iter(vec![
        event(1, changed(true, Some("CommandOrControl+Shift+F12"))),
        event(
            2,
            SubscriptionEvent::System(SystemEvent::AuditChainBroken { cursor: 1 }),
        ),
        event(3, changed(false, None)),
        event(4, changed(true, None)),
        StreamFrame::HostStopping {
            reason: StopReason::Requested,
        },
        event(5, changed(true, Some("Shift+F1"))),
    ]);
    let calls = parking_lot::Mutex::new(Vec::new());
    let end = follow_panic_changes(frames, |chord| {
        calls.lock().push(chord);
        async {}
    })
    .await;
    assert_eq!(end, FollowEnd::Stopped);
    assert_eq!(
        calls.into_inner(),
        vec![Some("CommandOrControl+Shift+F12".to_string()), None, None]
    );
    let behind = futures::stream::iter(vec![StreamFrame::InvalidSession]);
    assert_eq!(
        follow_panic_changes(behind, |_| async {}).await,
        FollowEnd::Resubscribe
    );
}

#[test]
fn main_thread_dispatch_failure_stays_an_error() {
    let failed = Err(ShortcutError::RecvError(std::sync::mpsc::RecvError));
    assert!(settle_registration("ctrl+shift+m", failed).is_err());
}

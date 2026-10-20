//! Which events become an OS notification.
//!
//! One decision for every frontend: the desktop shows the result as a
//! system notification, a TUI could ring the terminal bell. The inputs are
//! gathered by the host (preferences, Do Not Disturb, quiet hours, call and
//! presence state); this module only decides.
//!
//! Message notifications arrive here already filtered by Do Not Disturb,
//! quiet hours and the channel's notification level
//! (`should_emit_message_notification`); the rest are filtered here.

use rekindle_types::subscription_events::{
    FriendEvent, NotificationEvent, SocialEvent, SubscriptionEvent,
};

/// The user's state that decides whether to interrupt them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NotifyInputs {
    /// The user's "show notifications" preference.
    pub enabled: bool,
    /// Global Do Not Disturb.
    pub do_not_disturb: bool,
    /// The quiet-hours window is in effect now.
    pub quiet_hours_active: bool,
    /// The "silence notifications during calls" preference.
    pub in_call_dnd: bool,
    /// A call is connected.
    pub call_active: bool,
    /// Our own status is Busy.
    pub status_busy: bool,
}

/// An OS notification to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsNotification {
    pub title: String,
    pub body: String,
}

impl OsNotification {
    fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
        }
    }
}

/// Whether `event` can produce an OS notification at all, before the
/// (possibly costly) inputs are gathered.
#[must_use]
pub fn is_candidate(event: &SubscriptionEvent) -> bool {
    matches!(
        event,
        SubscriptionEvent::Notification(_)
            | SubscriptionEvent::Social(SocialEvent::EventReminder { .. })
            | SubscriptionEvent::Friend(FriendEvent::IdentityPinMismatch { .. })
    )
}

/// The OS notification for `event`, if any, given the user's state.
#[must_use]
pub fn os_notification_for(
    event: &SubscriptionEvent,
    inputs: &NotifyInputs,
) -> Option<OsNotification> {
    // A security alert: the user must learn that a friend's keys are not
    // signed by the identity they know, whatever silences the rest (the
    // "show notifications" preference, Do Not Disturb, quiet hours, Busy,
    // a call).
    if let SubscriptionEvent::Friend(FriendEvent::IdentityPinMismatch { display_name, .. }) = event
    {
        return Some(OsNotification::new(
            "Identity key mismatch",
            format!(
                "{display_name}'s keys are not signed by the identity you know. Nothing was \
                 sent; verify their safety number."
            ),
        ));
    }
    if !inputs.enabled || inputs.do_not_disturb {
        return None;
    }
    let in_call_silenced = inputs.in_call_dnd && inputs.call_active;
    match event {
        SubscriptionEvent::Notification(n) => match n {
            NotificationEvent::MessageReceived { title, body, .. } => {
                (!in_call_silenced).then(|| OsNotification::new(title, body))
            }
            NotificationEvent::SystemAlert { title, body } => {
                Some(OsNotification::new(title, body))
            }
            NotificationEvent::UpdateAvailable { version } => (!inputs.status_busy).then(|| {
                OsNotification::new(
                    "Update Available",
                    format!("Version {version} is available"),
                )
            }),
            NotificationEvent::CallIncoming {
                display_name,
                kind,
                is_group,
                ..
            } => (!inputs.quiet_hours_active).then(|| {
                let title = if *is_group {
                    format!("Incoming group {kind} call")
                } else {
                    format!("Incoming {kind} call")
                };
                OsNotification::new(title, display_name)
            }),
        },
        SubscriptionEvent::Social(SocialEvent::EventReminder {
            title,
            minutes_until_start,
            ..
        }) => (!inputs.quiet_hours_active && !in_call_silenced).then(|| {
            OsNotification::new(
                "Event Reminder",
                format!("{title} starts in {minutes_until_start} min"),
            )
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> NotifyInputs {
        NotifyInputs {
            enabled: true,
            ..NotifyInputs::default()
        }
    }

    fn message() -> SubscriptionEvent {
        SubscriptionEvent::Notification(NotificationEvent::MessageReceived {
            title: "Gamers".into(),
            body: "hi".into(),
            community_id: "c".into(),
            channel_id: "ch".into(),
            sound_ref: None,
        })
    }

    fn call() -> SubscriptionEvent {
        SubscriptionEvent::Notification(NotificationEvent::CallIncoming {
            call_id: "id".into(),
            from: "pk".into(),
            display_name: "Ada".into(),
            kind: "video".into(),
            expires_at_ms: 0,
            is_group: false,
        })
    }

    #[test]
    fn disabled_or_dnd_shows_nothing() {
        for inputs in [
            NotifyInputs::default(),
            NotifyInputs {
                do_not_disturb: true,
                ..on()
            },
        ] {
            assert_eq!(os_notification_for(&message(), &inputs), None);
            assert_eq!(os_notification_for(&call(), &inputs), None);
        }
    }

    #[test]
    fn calls_silence_messages_only_with_the_preference() {
        let in_call = NotifyInputs {
            call_active: true,
            ..on()
        };
        assert!(os_notification_for(&message(), &in_call).is_some());
        let silenced = NotifyInputs {
            in_call_dnd: true,
            ..in_call
        };
        assert_eq!(os_notification_for(&message(), &silenced), None);
    }

    #[test]
    fn quiet_hours_hold_back_calls_and_busy_holds_back_updates() {
        let quiet = NotifyInputs {
            quiet_hours_active: true,
            ..on()
        };
        assert_eq!(os_notification_for(&call(), &quiet), None);
        assert_eq!(
            os_notification_for(&call(), &on()),
            Some(OsNotification::new("Incoming video call", "Ada"))
        );
        let update = SubscriptionEvent::Notification(NotificationEvent::UpdateAvailable {
            version: "1.2".into(),
        });
        let busy = NotifyInputs {
            status_busy: true,
            ..on()
        };
        assert_eq!(os_notification_for(&update, &busy), None);
        assert!(os_notification_for(&update, &on()).is_some());
    }

    #[test]
    fn event_reminders_notify() {
        let reminder = SubscriptionEvent::Social(SocialEvent::EventReminder {
            community: "c".into(),
            event_id: "e".into(),
            title: "Raid night".into(),
            minutes_until_start: 15,
        });
        assert!(is_candidate(&reminder));
        assert_eq!(
            os_notification_for(&reminder, &on()),
            Some(OsNotification::new(
                "Event Reminder",
                "Raid night starts in 15 min"
            ))
        );
    }

    #[test]
    fn an_identity_pin_mismatch_always_notifies() {
        let mismatch = SubscriptionEvent::Friend(FriendEvent::IdentityPinMismatch {
            peer_key: "b".into(),
            display_name: "Bob".into(),
        });
        assert!(is_candidate(&mismatch));
        let quiet = NotifyInputs {
            quiet_hours_active: true,
            status_busy: true,
            ..on()
        };
        let shown = os_notification_for(&mismatch, &quiet).expect("a notification");
        assert_eq!(shown.title, "Identity key mismatch");
        assert!(shown.body.starts_with("Bob's keys"));
    }

    #[test]
    fn an_identity_pin_mismatch_is_shown_through_dnd_and_disabled_notifications() {
        let mismatch = SubscriptionEvent::Friend(FriendEvent::IdentityPinMismatch {
            peer_key: "b".into(),
            display_name: "Bob".into(),
        });
        for silenced in [
            NotifyInputs::default(),
            NotifyInputs {
                do_not_disturb: true,
                ..on()
            },
            NotifyInputs {
                do_not_disturb: true,
                in_call_dnd: true,
                call_active: true,
                ..NotifyInputs::default()
            },
        ] {
            assert!(
                os_notification_for(&mismatch, &silenced).is_some(),
                "silenced by {silenced:?}"
            );
            // Everything else stays silent.
            assert_eq!(os_notification_for(&message(), &silenced), None);
        }
    }
}

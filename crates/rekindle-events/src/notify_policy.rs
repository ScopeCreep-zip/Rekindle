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

use rekindle_types::preferences::NotificationContent;
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
    /// What a notification may show of who and what
    /// (`Preferences.notification_content`).
    pub content: NotificationContent,
}

/// The title shown for a message when the content setting hides its sender.
const HIDDEN_MESSAGE_TITLE: &str = "New message";

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
        let whose = if inputs.content == NotificationContent::Nothing {
            "A contact's".to_string()
        } else {
            format!("{display_name}'s")
        };
        return Some(OsNotification::new(
            "Identity key mismatch",
            format!(
                "{whose} keys are not signed by the identity you know. Nothing was \
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
                // `title` names the sender, or the channel and community for
                // a community message: a name, not content, so `nameOnly`
                // keeps it and drops only the body.
                (!in_call_silenced).then(|| match inputs.content {
                    NotificationContent::NameAndMessage => OsNotification::new(title, body),
                    NotificationContent::NameOnly => OsNotification::new(title, ""),
                    NotificationContent::Nothing => OsNotification::new(HIDDEN_MESSAGE_TITLE, ""),
                })
            }
            // An alert carries no message content; one about a contact
            // names them in `subject`, and under `nothing` it is shown
            // with its unnamed text.
            NotificationEvent::SystemAlert {
                title,
                body,
                subject,
            } => Some(match (inputs.content, subject) {
                (NotificationContent::Nothing, Some(subject)) => {
                    OsNotification::new(title, subject.unnamed_body.clone())
                }
                _ => OsNotification::new(title, body),
            }),
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
                let shown = if inputs.content == NotificationContent::Nothing {
                    ""
                } else {
                    display_name.as_str()
                };
                OsNotification::new(title, shown)
            }),
        },
        SubscriptionEvent::Social(SocialEvent::EventReminder {
            title,
            minutes_until_start,
            ..
        }) => (!inputs.quiet_hours_active && !in_call_silenced).then(|| {
            // An event's title is what its organiser wrote, so it counts as
            // content: shown only under `nameAndMessage`.
            let body = if inputs.content == NotificationContent::NameAndMessage {
                format!("{title} starts in {minutes_until_start} min")
            } else {
                format!("An event starts in {minutes_until_start} min")
            };
            OsNotification::new("Event Reminder", body)
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_content_setting_hides_the_message_or_everything() {
        let shown =
            |content| os_notification_for(&message(), &NotifyInputs { content, ..on() }).unwrap();
        let full = shown(NotificationContent::NameAndMessage);
        assert_eq!(
            (full.title.as_str(), full.body.is_empty()),
            ("Gamers", false)
        );
        let name = shown(NotificationContent::NameOnly);
        assert_eq!((name.title.as_str(), name.body.as_str()), ("Gamers", ""));
        let nothing = shown(NotificationContent::Nothing);
        assert_eq!(
            (nothing.title.as_str(), nothing.body.as_str()),
            (HIDDEN_MESSAGE_TITLE, "")
        );
        assert_eq!(
            NotifyInputs::default().content,
            NotificationContent::NameOnly,
            "the conservative default (decided from research 2026-10-24)"
        );
        let call = SubscriptionEvent::Notification(NotificationEvent::CallIncoming {
            call_id: "c".into(),
            from: "p".into(),
            display_name: "Rae".into(),
            kind: "voice".into(),
            expires_at_ms: 0,
            is_group: false,
        });
        let hidden = NotifyInputs {
            content: NotificationContent::Nothing,
            ..on()
        };
        assert_eq!(os_notification_for(&call, &hidden).unwrap().body, "");
    }

    /// Every alert kind that names a contact, as its producer builds it,
    /// and the generic ones: under `nothing` no display name is shown.
    #[test]
    fn no_display_name_is_shown_under_nothing_for_any_alert() {
        let name = "Rae Winters";
        let alerts = [
            NotificationEvent::contact_alert(
                "Friend request declined",
                name,
                format!("{name} declined your friend request"),
                "A contact declined your friend request",
            ),
            NotificationEvent::contact_alert(
                "Session Reset Declined",
                name,
                format!("{name} declined your secure session reset request."),
                "A contact declined your secure session reset request.",
            ),
            NotificationEvent::contact_alert(
                "Secure session re-established",
                name,
                format!("A new secure session with {name} is active."),
                "A new secure session with a contact is active.",
            ),
            NotificationEvent::SystemAlert {
                title: "Network".into(),
                body: "Veilid network attached".into(),
                subject: None,
            },
        ];
        let events: Vec<SubscriptionEvent> = alerts
            .into_iter()
            .map(SubscriptionEvent::Notification)
            .chain([
                message_from(name),
                call_from(name),
                SubscriptionEvent::Friend(FriendEvent::IdentityPinMismatch {
                    peer_key: "pk".into(),
                    display_name: name.into(),
                }),
                SubscriptionEvent::Social(SocialEvent::EventReminder {
                    community: "c".into(),
                    event_id: "e".into(),
                    title: format!("{name}'s raid"),
                    minutes_until_start: 5,
                }),
            ])
            .collect();
        let nothing = NotifyInputs {
            content: NotificationContent::Nothing,
            ..on()
        };
        for event in &events {
            let shown = os_notification_for(event, &nothing).unwrap();
            assert!(
                !shown.title.contains(name) && !shown.body.contains(name),
                "{event:?} showed {shown:?}"
            );
        }
        // Under nameOnly a contact alert keeps the name (an alert has no
        // message content).
        let name_only = NotifyInputs {
            content: NotificationContent::NameOnly,
            ..on()
        };
        let shown = os_notification_for(&events[0], &name_only).unwrap();
        assert!(shown.body.contains(name));
    }

    /// `nameOnly`: a community message keeps its title (the channel and
    /// community name, not content) and drops the body; a reminder drops
    /// the event title, which is content.
    #[test]
    fn name_only_keeps_channel_names_and_hides_event_titles() {
        let name_only = NotifyInputs {
            content: NotificationContent::NameOnly,
            ..on()
        };
        let channel = os_notification_for(&message(), &name_only).unwrap();
        assert_eq!(
            (channel.title.as_str(), channel.body.as_str()),
            ("Gamers", "")
        );
        let reminder = SubscriptionEvent::Social(SocialEvent::EventReminder {
            community: "c".into(),
            event_id: "e".into(),
            title: "Secret raid".into(),
            minutes_until_start: 5,
        });
        let shown = os_notification_for(&reminder, &name_only).unwrap();
        assert!(!shown.body.contains("Secret raid"), "{shown:?}");
        let full = NotifyInputs {
            content: NotificationContent::NameAndMessage,
            ..on()
        };
        assert!(os_notification_for(&reminder, &full)
            .unwrap()
            .body
            .contains("Secret raid"));
    }

    fn message_from(sender: &str) -> SubscriptionEvent {
        SubscriptionEvent::Notification(NotificationEvent::MessageReceived {
            title: sender.into(),
            body: "hi".into(),
            community_id: "c".into(),
            channel_id: "ch".into(),
            sound_ref: None,
        })
    }

    fn call_from(name: &str) -> SubscriptionEvent {
        SubscriptionEvent::Notification(NotificationEvent::CallIncoming {
            call_id: "id".into(),
            from: "pk".into(),
            display_name: name.into(),
            kind: "voice".into(),
            expires_at_ms: 0,
            is_group: false,
        })
    }

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

    /// Our own sent message (plan E6.11) is never a notification: it is not
    /// a candidate, and with every alert on it still shows nothing.
    #[test]
    fn our_own_sent_message_never_notifies() {
        use rekindle_types::subscription_events::ChannelMessageEvent;
        let sent = SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DirectMessageSent {
            conversation_id: "pk".into(),
            reply_to_id: None,
            message: rekindle_types::dto::dm::DmMessage {
                id: 1,
                message_id: Some("01900000-0000-7000-8000-000000000002".into()),
                sender_id: "me".into(),
                body: "hi".into(),
                timestamp: 1,
                is_own: true,
                send_state: rekindle_types::message::SendState::Queued,
                history_gap_through: None,
                expire_seconds: 0,
                expire_at: None,
                timer_notice_seconds: None,
            },
        });
        assert!(!is_candidate(&sent));
        let all_on = NotifyInputs {
            content: NotificationContent::NameAndMessage,
            ..on()
        };
        assert_eq!(os_notification_for(&sent, &all_on), None);
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
        let full = NotifyInputs {
            content: NotificationContent::NameAndMessage,
            ..on()
        };
        assert_eq!(
            os_notification_for(&reminder, &full),
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

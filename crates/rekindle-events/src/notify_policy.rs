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
                ..
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
#[path = "notify_policy_tests.rs"]
mod tests;

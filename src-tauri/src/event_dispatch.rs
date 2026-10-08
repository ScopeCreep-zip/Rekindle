//! Every event the backend sends to a webview, and who receives it.
//!
//! An event is a typed [`WebviewEvent`]. Emitters push it onto one queue;
//! one dispatch task drains the queue, works out the event's
//! [`Audience`] and hands it to [`crate::event_router::WebviewRouter`],
//! which sends it on each audience window's channel. Nothing in
//! `src-tauri` uses Tauri's `Emitter` (ADR 0007, enforced by
//! `cargo xtask check-no-emitter`).
//!
//! The audience is an exhaustive match: a new event variant does not
//! compile until it says which windows it is for. For a
//! [`SubscriptionEvent`] the audience follows its Tier 1
//! [`EventScope`], so the desktop routes the same way the daemon and the
//! TUI do.
//!
//! Journaled events ([`emit_journaled`]) also go into the event journal
//! with a sequence number. A window that reloads asks for the journaled
//! events it missed (`subscribe_events` with its last sequence number);
//! nothing else is ever replayed.

use std::sync::Arc;

use rekindle_types::subscription_events::{
    ChannelMessageEvent, EventScope, SubscriptionEvent, TypingContext, TypingEvent,
};
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tokio::sync::mpsc;

use crate::channels::{ChatEvent, CommunityEvent};
use crate::deep_links::DeepLinkRequest;
use crate::event_router::{Audience, OutboundEnvelope};
use crate::state::{AppState, SharedState};
use crate::window_labels::{self as labels, WindowKind};
use crate::windows::SettingsTab;

/// Everything the backend sends to a webview.
#[derive(Debug, Clone)]
pub enum WebviewEvent {
    /// The shared vocabulary every frontend observes.
    Subscription(SubscriptionEvent),
    /// The app lifecycle state machine moved.
    Lifecycle {
        state: rekindle_lifecycle::LifecycleState,
        at_ms: i64,
    },
    /// An OS deep link is waiting for the user's consent.
    DeepLink(DeepLinkRequest),
    /// Our own profile (name, avatar) changed.
    ProfileUpdated,
    /// Switch the open settings window to a tab.
    SettingsSwitchTab(SettingsTab),
    /// A community channel message in the desktop's legacy shape, with the
    /// community it belongs to when the emitter knows it. Plan step E
    /// moves these emitters onto `ChannelMessageEvent::New`.
    ChannelChat {
        community_id: Option<String>,
        event: ChatEvent,
    },
    /// A community-window event in the desktop's legacy shape.
    Community(CommunityEvent),
}

/// Lifecycle payload, as the login window reads it.
#[derive(Serialize)]
struct LifecyclePayload {
    state: rekindle_lifecycle::LifecycleState,
    at_ms: i64,
}

impl WebviewEvent {
    /// The frontend channel whose handlers consume this event.
    #[must_use]
    pub fn channel(&self) -> &'static str {
        match self {
            Self::Subscription(e) => subscription_channel(e),
            Self::Lifecycle { .. } => "lifecycle-event",
            Self::DeepLink(_) => "deep-link-action",
            Self::ProfileUpdated => "profile-updated",
            Self::SettingsSwitchTab(_) => "settings-switch-tab",
            Self::ChannelChat { .. } => "chat-event",
            Self::Community(_) => "community-event",
        }
    }

    fn payload(&self) -> Result<serde_json::Value, serde_json::Error> {
        match self {
            Self::Subscription(e) => serde_json::to_value(e),
            Self::Lifecycle { state, at_ms } => serde_json::to_value(LifecyclePayload {
                state: *state,
                at_ms: *at_ms,
            }),
            Self::DeepLink(r) => serde_json::to_value(r),
            Self::ProfileUpdated => Ok(serde_json::Value::Null),
            Self::SettingsSwitchTab(tab) => serde_json::to_value(tab),
            Self::ChannelChat { event, .. } => serde_json::to_value(event),
            Self::Community(e) => serde_json::to_value(e),
        }
    }

    /// The windows this event is for.
    #[must_use]
    pub fn audience(&self) -> Audience {
        match self {
            Self::Subscription(e) => subscription_audience(e),
            Self::Lifecycle { .. } => Audience::kinds(&WindowKind::ALL),
            Self::DeepLink(_) => Audience::labels([labels::BUDDY_LIST.to_owned()]),
            Self::ProfileUpdated => Audience::post_auth(),
            Self::SettingsSwitchTab(_) => Audience::labels([labels::SETTINGS.to_owned()]),
            // Community windows switch between communities in place, so a
            // community's events go to every one of them (each filters by
            // its selection) and to the buddy list's community list. A
            // channel message whose community the emitter does not know
            // can only go there too.
            Self::ChannelChat { .. } | Self::Community(_) => communities(),
        }
    }

    /// The envelope a window receives, or `None` if the payload does not
    /// serialize (logged; such an event reaches no one).
    #[must_use]
    pub fn envelope(&self, seq: Option<u64>) -> Option<OutboundEnvelope> {
        match self.payload() {
            Ok(payload) => Some(OutboundEnvelope {
                seq,
                channel: self.channel(),
                payload,
            }),
            Err(e) => {
                tracing::warn!(channel = self.channel(), error = %e, "event payload did not serialize");
                None
            }
        }
    }
}

/// The buddy list and every community window.
fn communities() -> Audience {
    Audience::labels([labels::BUDDY_LIST.to_owned()]).with_kinds(&[WindowKind::Community])
}

/// The buddy list only: device-wide UI with a single owner.
fn buddy_list() -> Audience {
    Audience::labels([labels::BUDDY_LIST.to_owned()])
}

fn subscription_audience(event: &SubscriptionEvent) -> Audience {
    match event {
        // One OS notification, one inbox, one ring: the buddy list owns them.
        SubscriptionEvent::Notification(_)
        | SubscriptionEvent::ChannelMessage(ChannelMessageEvent::ConversationFocusRequested {
            ..
        }) => buddy_list(),
        // The call's own window, the buddy list, and the chat and DM
        // windows (each shows the call with its peer).
        SubscriptionEvent::Call(e) => buddy_list()
            .with_labels(labels::call_label_for(e.call_id()))
            .with_kinds(&[WindowKind::Chat, WindowKind::Dm]),
        _ => match event.scope() {
            EventScope::Device => Audience::post_auth(),
            EventScope::Community(_) | EventScope::CommunityJoin(_) => communities(),
            EventScope::Peer(peer) => {
                let audience = buddy_list().with_labels(labels::peer_labels(&peer));
                // A DM call's media and voice state also drive its call window.
                if matches!(event, SubscriptionEvent::Voice(_)) {
                    audience.with_kinds(&[WindowKind::Call])
                } else {
                    audience
                }
            }
            EventScope::Conversation(record) => {
                buddy_list().with_labels(labels::conversation_label(&record))
            }
            EventScope::Call(id) => buddy_list().with_labels(labels::call_label_for(&id)),
        },
    }
}

/// Which frontend channel a [`SubscriptionEvent`] is consumed on.
///
/// Many families share a channel because the frontend's handlers are
/// organized by window, not by family.
fn subscription_channel(event: &SubscriptionEvent) -> &'static str {
    match event {
        SubscriptionEvent::Presence(_) => "presence-event",
        SubscriptionEvent::Voice(_) => "voice-event",
        // `ChannelMessage` and `Typing` split by context: the community
        // half goes to the community dispatcher, the direct half to the
        // chat handlers.
        SubscriptionEvent::ChannelMessage(
            ChannelMessageEvent::New { .. }
            | ChannelMessageEvent::Edited { .. }
            | ChannelMessageEvent::Deleted { .. },
        )
        | SubscriptionEvent::Typing(
            TypingEvent::Started {
                context: TypingContext::Channel { .. },
                ..
            }
            | TypingEvent::Stopped {
                context: TypingContext::Channel { .. },
                ..
            },
        )
        | SubscriptionEvent::Membership(_)
        | SubscriptionEvent::Governance(_)
        | SubscriptionEvent::Crypto(_)
        | SubscriptionEvent::Social(_)
        // System events (announcements, raids, automod, lockdowns, sync,
        // audit) are handled by the community dispatcher.
        | SubscriptionEvent::System(_)
        | SubscriptionEvent::UnreadChanged { .. } => "community-event",
        SubscriptionEvent::ChannelMessage(_)
        | SubscriptionEvent::Typing(_)
        | SubscriptionEvent::Call(_)
        | SubscriptionEvent::Friend(_) => "chat-event",
        SubscriptionEvent::Notification(_) => "notification-event",
        SubscriptionEvent::Network(_) => "network-status",
    }
}

/// An event waiting for the dispatch task.
struct Queued {
    seq: Option<u64>,
    event: WebviewEvent,
}

/// The single dispatch queue. Emitters push; one task drains.
pub struct EventDispatch {
    tx: mpsc::UnboundedSender<Queued>,
    rx_holder: parking_lot::Mutex<Option<mpsc::UnboundedReceiver<Queued>>>,
}

impl EventDispatch {
    /// A queue that buffers until [`spawn_dispatch_loop`] takes it.
    #[must_use]
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx,
            rx_holder: parking_lot::Mutex::new(Some(rx)),
        }
    }

    fn take_receiver(&self) -> Option<mpsc::UnboundedReceiver<Queued>> {
        self.rx_holder.lock().take()
    }

    fn enqueue(&self, seq: Option<u64>, event: WebviewEvent) {
        let _ = self.tx.send(Queued { seq, event });
    }
}

impl Default for EventDispatch {
    fn default() -> Self {
        Self::new()
    }
}

/// Start the dispatch task: each event goes to the windows in its
/// audience, and an event the notification policy picks also becomes an
/// OS notification (`os_notify`).
pub fn spawn_dispatch_loop(app: AppHandle, state: &Arc<AppState>) {
    let Some(mut rx) = state.event_dispatch.take_receiver() else {
        tracing::warn!("spawn_dispatch_loop: receiver already taken — duplicate setup?");
        return;
    };
    let state = Arc::clone(state);
    tauri::async_runtime::spawn(async move {
        while let Some(Queued { seq, event }) = rx.recv().await {
            if let WebviewEvent::Subscription(e) = &event {
                crate::os_notify::notify(&app, &state, e);
            }
            if let Some(envelope) = event.envelope(seq) {
                state.event_router.deliver(&envelope, &event.audience());
            }
        }
        tracing::debug!("event-dispatch loop exited (channel closed)");
    });
}

/// Send `event` to its audience, live only.
pub fn emit(app: &AppHandle, event: WebviewEvent) {
    if let Some(state) = app.try_state::<SharedState>() {
        emit_from_state(&state, event);
    } else {
        tracing::warn!(
            channel = event.channel(),
            "emit before state is managed — dropped"
        );
    }
}

/// Send `event` to its audience, for callers that hold the state.
pub fn emit_from_state(state: &AppState, event: WebviewEvent) {
    state.event_dispatch.enqueue(None, event);
}

/// Journal `event` (so a reloading window can recover it), then send it.
pub fn emit_journaled(state: &AppState, event: WebviewEvent) {
    let seq = state.event_journal.append(event.clone());
    state.event_dispatch.enqueue(Some(seq), event);
}

/// Send a shared-vocabulary event, live.
pub fn emit_subscription(app: &AppHandle, event: &SubscriptionEvent) {
    emit(app, WebviewEvent::Subscription(event.clone()));
}

/// Send a voice event.
pub fn emit_voice(app: &AppHandle, event: rekindle_types::subscription_events::VoiceEvent) {
    emit(
        app,
        WebviewEvent::Subscription(SubscriptionEvent::Voice(event)),
    );
}

/// Send a community membership event.
pub fn emit_membership(
    app: &AppHandle,
    event: rekindle_types::subscription_events::MembershipEvent,
) {
    emit(
        app,
        WebviewEvent::Subscription(SubscriptionEvent::Membership(event)),
    );
}

/// Send a call-signalling event.
pub fn emit_call(app: &AppHandle, event: rekindle_types::subscription_events::CallEvent) {
    emit(
        app,
        WebviewEvent::Subscription(SubscriptionEvent::Call(event)),
    );
}

/// Send a device-level notification.
pub fn emit_notification(
    app: &AppHandle,
    event: rekindle_types::subscription_events::NotificationEvent,
) {
    emit(
        app,
        WebviewEvent::Subscription(SubscriptionEvent::Notification(event)),
    );
}

/// Send a legacy community-window event.
pub fn emit_community(app: &AppHandle, event: CommunityEvent) {
    emit(app, WebviewEvent::Community(event));
}

/// The journaled events after `since` that a window labelled `label` would
/// have received, as envelopes.
#[must_use]
pub fn replay_for(state: &AppState, label: &str, since: u64) -> Vec<OutboundEnvelope> {
    state
        .event_journal
        .replay_since(since)
        .into_iter()
        .filter(|entry| entry.event.audience().includes(label))
        .filter_map(|entry| entry.event.envelope(Some(entry.cursor)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_types::subscription_events::{
        CallEvent, MembershipEvent, NotificationEvent, PresenceEvent, PresenceSnapshot, VoiceEvent,
        VoiceScope,
    };

    const PK: &str = "3f1a9c0b7e2d4f6a8b1c3e5d7f9a0b2c4d6e8f0a1b3c5d7e9f1a2b4c6d8e0f1a";
    const CALL: &str = "0123456789abcdef0123456789abcdef";
    const REC: &str = "VLD0:um7m8HxBluv6XceSaB3dK9Lq0ZpWtYv1NcRe4Gh7Jf2";

    fn sub(e: SubscriptionEvent) -> Audience {
        WebviewEvent::Subscription(e).audience()
    }

    #[test]
    fn notifications_belong_to_the_buddy_list_only() {
        let a = sub(SubscriptionEvent::Notification(
            NotificationEvent::SystemAlert {
                title: "t".into(),
                body: "b".into(),
            },
        ));
        assert!(a.includes("buddy-list"));
        assert!(!a.includes("chat-3f1a9c0b7e2d4f6a"));
        assert!(!a.includes("settings"));
    }

    #[test]
    fn a_direct_message_reaches_its_chat_window_only() {
        let a = sub(SubscriptionEvent::ChannelMessage(
            ChannelMessageEvent::DirectMessageAcknowledged {
                message_id: 1,
                conversation_id: PK.into(),
            },
        ));
        assert!(a.includes("buddy-list"));
        assert!(a.includes("chat-3f1a9c0b7e2d4f6a"));
        assert!(!a.includes("chat-0000000000000000"));
        assert!(!a.includes("community-browser"));
    }

    #[test]
    fn group_dms_reach_their_dm_window() {
        let a = sub(SubscriptionEvent::ChannelMessage(
            ChannelMessageEvent::DirectMessageReceived {
                peer_key: PK.into(),
                timestamp: 1,
                sender_name: None,
                body: None,
                decryption_failed: false,
                automod_blurred: false,
                conversation_id: REC.into(),
                server_message_id: None,
                reply_to_id: None,
            },
        ));
        assert!(a.includes(&labels::conversation_label(REC).unwrap()));
        assert!(!a.includes("chat-3f1a9c0b7e2d4f6a"));
    }

    #[test]
    fn joins_reach_every_community_window() {
        let a = sub(SubscriptionEvent::Membership(
            MembershipEvent::JoinProgress {
                community: REC.into(),
                stage: String::new(),
                status: String::new(),
            },
        ));
        assert!(a.includes("community-browser"));
        assert!(a.includes("buddy-list"));
        assert!(!a.includes("settings"));
    }

    #[test]
    fn calls_reach_their_window_and_conversation_windows() {
        let a = sub(SubscriptionEvent::Call(CallEvent::Ringing {
            call_id: CALL.into(),
        }));
        assert!(a.includes("buddy-list"));
        assert!(a.includes("call-0123456789ab"));
        assert!(!a.includes("call-ffffffffffff"));
        assert!(a.includes("chat-3f1a9c0b7e2d4f6a"));
        assert!(!a.includes("community-browser"));
        let voice = sub(SubscriptionEvent::Voice(VoiceEvent::LocalJoined {
            scope: VoiceScope::Dm {
                peer_key: PK.into(),
            },
        }));
        assert!(voice.includes("call-0123456789ab"));
    }

    #[test]
    fn device_events_reach_every_post_login_window() {
        let a = sub(SubscriptionEvent::Presence(PresenceEvent::SelfChanged {
            public_key: PK.into(),
            snapshot: PresenceSnapshot::default(),
        }));
        assert!(a.includes("settings"));
        assert!(!a.includes("login"));
    }

    #[test]
    fn system_events_ride_the_community_channel() {
        let e = WebviewEvent::Subscription(SubscriptionEvent::System(
            rekindle_types::subscription_events::SystemEvent::RaidAlert {
                community: REC.into(),
                active: true,
            },
        ));
        assert_eq!(e.channel(), "community-event");
    }
}

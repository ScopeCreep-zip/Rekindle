//! Every event the backend sends to a webview, and who receives it.
//!
//! An event is a typed [`WebviewEvent`]. Emitters push it onto one queue;
//! one dispatch task drains the queue, works out the event's
//! [`Audience`] and hands it to [`crate::event_router::WebviewRouter`],
//! which sends it on each audience window's channel. Nothing in
//! `src-tauri` uses Tauri's `Emitter` (ADR 0013, enforced by
//! `cargo xtask check-no-emitter`).
//!
//! The audience is an exhaustive match: a new event variant does not
//! compile until it says which windows it is for. For a
//! [`SubscriptionEvent`] the audience follows its Tier 1
//! [`EventScope`], so the desktop routes the same way the daemon and the
//! TUI do.
//!
//! A [`SubscriptionEvent`] is journaled by the in-process host (plan
//! D3.2), which gives it a cursor; [`spawn_host_stream`] delivers the
//! host's stream to the windows, and a window that reloads asks the host
//! for what followed its last cursor (`subscribe_events`). The shell-only
//! variants (`Lifecycle`, `DeepLink`, `SettingsSwitchTab`)
//! and the legacy `ChannelChat`/`Community` shapes are delivered live from
//! the local queue and never replayed; a reloading window reads that
//! history through `hydrateState()`.

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
    pub fn envelope(
        &self,
        cursor: Option<rekindle_types::subscription_events::Cursor>,
    ) -> Option<OutboundEnvelope> {
        match self.payload() {
            Ok(payload) => Some(OutboundEnvelope {
                cursor,
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
        // One OS notification, one inbox, one ring, one review of an action
        // the backend began: the buddy list owns them.
        SubscriptionEvent::Notification(_)
        | SubscriptionEvent::IntentPending(_)
        | SubscriptionEvent::ChannelMessage(ChannelMessageEvent::ConversationFocusRequested {
            ..
        }) => buddy_list(),
        // The call's own window, the buddy list, and the chat windows
        // (each shows the call with its peer).
        SubscriptionEvent::Call(e) => buddy_list()
            .with_labels(labels::call_label_for(e.call_id()))
            .with_kinds(&[WindowKind::Chat]),
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
        | SubscriptionEvent::UnreadChanged {
            context: rekindle_types::subscription_events::UnreadContext::Channel { .. },
            ..
        } => "community-event",
        // A DM's or the friend requests' unread count goes to the friend
        // and chat handlers.
        SubscriptionEvent::UnreadChanged { .. }
        | SubscriptionEvent::ChannelMessage(_)
        | SubscriptionEvent::Typing(_)
        | SubscriptionEvent::Call(_)
        | SubscriptionEvent::Friend(_) => "chat-event",
        SubscriptionEvent::Notification(_) => "notification-event",
        SubscriptionEvent::IntentPending(_) => "intent-event",
        SubscriptionEvent::Network(_) => "network-status",
    }
}

/// How many local events wait for the dispatch task.
const QUEUE_CAPACITY: usize = 4096;

/// The local dispatch queue (shell-only and legacy events). Emitters push;
/// one task drains. It is bounded: when it is full the event is dropped and
/// every window is told to reload (its history is in the database).
pub struct EventDispatch {
    tx: mpsc::Sender<WebviewEvent>,
    rx_holder: parking_lot::Mutex<Option<mpsc::Receiver<WebviewEvent>>>,
    overflowed: std::sync::atomic::AtomicBool,
}

impl EventDispatch {
    /// A queue that buffers until [`spawn_dispatch_loop`] takes it.
    #[must_use]
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(QUEUE_CAPACITY);
        Self {
            tx,
            rx_holder: parking_lot::Mutex::new(Some(rx)),
            overflowed: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn take_receiver(&self) -> Option<mpsc::Receiver<WebviewEvent>> {
        self.rx_holder.lock().take()
    }

    fn enqueue(&self, event: WebviewEvent) {
        if let Err(mpsc::error::TrySendError::Full(event)) = self.tx.try_send(event) {
            tracing::warn!(
                channel = event.channel(),
                "event queue full; windows reload"
            );
            self.overflowed
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

impl Default for EventDispatch {
    fn default() -> Self {
        Self::new()
    }
}

/// Start the local dispatch task: each event goes to the windows in its
/// audience. A dropped event (full queue) makes every window reload.
pub fn spawn_dispatch_loop(state: &Arc<AppState>) {
    let Some(mut rx) = state.event_dispatch.take_receiver() else {
        tracing::warn!("spawn_dispatch_loop: receiver already taken — duplicate setup?");
        return;
    };
    let state = Arc::clone(state);
    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            if state
                .event_dispatch
                .overflowed
                .swap(false, std::sync::atomic::Ordering::Relaxed)
            {
                state
                    .event_router
                    .deliver_all(&OutboundEnvelope::invalid_session());
            }
            if let Some(envelope) = event.envelope(None) {
                state.event_router.deliver(&envelope, &event.audience());
            }
        }
        tracing::debug!("event-dispatch loop exited (channel closed)");
    });
}

/// Deliver the host's event stream to the windows (plan D3.2): each event
/// to its audience with its cursor, and each event the notification policy
/// picks as an OS notification. `InvalidSession` (a new epoch at login or
/// logout, or this task fell behind) makes every window reload; the stream
/// is then taken again, live.
pub fn spawn_host_stream(
    app: AppHandle,
    state: &Arc<AppState>,
    host: Arc<rekindle_node::host::Host>,
) {
    use rekindle_ipc::HostApi as _;
    use rekindle_types::subscription_events::StreamFrame;
    let state = Arc::clone(state);
    tauri::async_runtime::spawn(async move {
        loop {
            let mut stream = match host.subscribe(None).await {
                Ok(stream) => stream,
                Err(e) => {
                    tracing::error!(error = %e, "the host's event stream did not open");
                    return;
                }
            };
            while let Some(frame) = stream.next().await {
                match frame {
                    StreamFrame::Event(sequenced) => {
                        crate::state_helpers::project_own(&state, &sequenced.event);
                        if let rekindle_types::subscription_events::SubscriptionEvent::Presence(
                            rekindle_types::subscription_events::PresenceEvent::SelfChanged {
                                snapshot,
                                ..
                            },
                        ) = &sequenced.event
                        {
                            crate::services::community_presence_runtime::fan_out_own_presence(
                                &state, snapshot,
                            );
                        }
                        crate::os_notify::notify(&app, &state, &sequenced.event);
                        // A review the backend began is answered on the
                        // buddy list: bring it forward (never recreated,
                        // which would drop the review).
                        if matches!(sequenced.event, SubscriptionEvent::IntentPending(_)) {
                            crate::windows::surface_buddy_list(&app);
                        }
                        let event = WebviewEvent::Subscription(sequenced.event);
                        if let Some(envelope) = event.envelope(Some(sequenced.cursor)) {
                            state.event_router.deliver(&envelope, &event.audience());
                        }
                    }
                    StreamFrame::InvalidSession => state
                        .event_router
                        .deliver_all(&OutboundEnvelope::invalid_session()),
                    StreamFrame::Resumed(_) => {}
                }
            }
            // The stream ended after an `InvalidSession` (this task fell
            // behind): take it again, live.
        }
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

/// Send `event` to its audience, for callers that hold the state: a
/// [`SubscriptionEvent`] through the host's journal, the rest through the
/// local queue.
pub fn emit_from_state(state: &AppState, event: WebviewEvent) {
    match event {
        WebviewEvent::Subscription(event) => state.host.publish(event),
        other => state.event_dispatch.enqueue(other),
    }
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

/// A reloading window's replay from `from` (plan D3.2): the journaled
/// events after it that were addressed to `label`, then `Resumed`; or
/// `InvalidSession`, when the host cannot serve the cursor.
#[must_use]
pub fn replay_for(
    host: &rekindle_node::host::Host,
    label: &str,
    from: rekindle_types::subscription_events::Cursor,
) -> Vec<OutboundEnvelope> {
    use rekindle_types::subscription_events::StreamFrame;
    host.replay(from)
        .into_iter()
        .filter_map(|frame| match frame {
            StreamFrame::Event(sequenced) => {
                let event = WebviewEvent::Subscription(sequenced.event);
                event
                    .audience()
                    .includes(label)
                    .then(|| event.envelope(Some(sequenced.cursor)))
                    .flatten()
            }
            StreamFrame::Resumed(cursor) => Some(OutboundEnvelope::resumed(cursor)),
            StreamFrame::InvalidSession => Some(OutboundEnvelope::invalid_session()),
        })
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
            ChannelMessageEvent::DirectMessageStateChanged {
                message_id: "01900000-0000-7000-8000-000000000002".into(),
                conversation_id: PK.into(),
                send_state: rekindle_types::message::SendState::Sent,
            },
        ));
        assert!(a.includes("buddy-list"));
        assert!(a.includes("chat-3f1a9c0b7e2d4f6a"));
        assert!(!a.includes("chat-0000000000000000"));
        assert!(!a.includes("community-browser"));
    }

    #[test]
    fn group_dms_reach_their_group_chat_window() {
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
                is_own: false,
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
        // Our profile too, on the presence channel (it replaced the
        // shell's `profile-updated`).
        let profile = SubscriptionEvent::Presence(PresenceEvent::SelfProfileChanged {
            public_key: PK.into(),
            display_name: None,
            avatar_changed: true,
        });
        assert!(sub(profile.clone()).includes("settings"));
        assert!(!sub(profile.clone()).includes("login"));
        assert_eq!(
            WebviewEvent::Subscription(profile).channel(),
            "presence-event"
        );
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

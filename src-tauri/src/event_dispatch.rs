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
//! compile until it says which windows it is for. A [`SubscriptionEvent`]'s
//! audience follows its Tier 1 [`EventScope`], as on the daemon and TUI.
//!
//! A [`SubscriptionEvent`] is journaled by the in-process host (plan
//! D3.2), which gives it a cursor; [`spawn_host_stream`] delivers the
//! host's stream to the windows, and a window that reloads asks the host
//! for what followed its last cursor (`subscribe_events`). The shell-only
//! variants (`Lifecycle`, `PairingLinkRefused`, `SettingsSwitchTab`)
//! and the legacy `Community` shape are delivered live from
//! the local queue and never replayed; a reloading window reads that
//! history through `hydrateState()`.

use std::sync::Arc;

use rekindle_types::subscription_events::{
    ChannelMessageEvent, EventScope, FriendEvent, NetworkEvent, SubscriptionEvent, SystemEvent,
    TypingContext, TypingEvent,
};
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tokio::sync::mpsc;

use crate::channels::CommunityEvent;
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
    /// A pairing link arrived from the OS and was refused: pairing codes
    /// are accepted only in Settings (plan E3.1, U1).
    PairingLinkRefused,
    /// Switch the open settings window to a tab.
    SettingsSwitchTab(SettingsTab),
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
            Self::PairingLinkRefused => "pairing-link-refused",
            Self::SettingsSwitchTab(_) => "settings-switch-tab",
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
            Self::PairingLinkRefused => Ok(serde_json::Value::Null),
            Self::SettingsSwitchTab(tab) => serde_json::to_value(tab),
            Self::Community(e) => serde_json::to_value(e),
        }
    }

    /// The windows this event is for.
    #[must_use]
    pub fn audience(&self) -> Audience {
        match self {
            Self::Subscription(e) => subscription_audience(e),
            Self::Lifecycle { .. } => Audience::kinds(&WindowKind::ALL),
            Self::PairingLinkRefused => Audience::labels([labels::BUDDY_LIST.to_owned()]),
            Self::SettingsSwitchTab(_) => Audience::labels([labels::SETTINGS.to_owned()]),
            // Community windows switch between communities in place, so a
            // community's events go to every one of them (each filters by
            // its selection) and to the buddy list's community list.
            Self::Community(_) => communities(),
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
        // Every window shows the skin, the login window included (plan
        // E7.7.2): each swaps its stylesheet.
        SubscriptionEvent::System(SystemEvent::SkinChanged { .. }) => {
            Audience::kinds(&WindowKind::ALL)
        }
        // "Keep this skin?" is asked where skins are chosen: Settings →
        // Appearance renders it from the payload (plan E7.7.2). The quick
        // wipe's state shows in Settings → Privacy, where it is armed (plan
        // E7.1); the shell's own registration follows it in `shell_session`.
        SubscriptionEvent::System(
            SystemEvent::SkinTrialChanged { .. } | SystemEvent::QuickWipeChanged { .. },
        ) => Audience::kinds(&[WindowKind::Settings]),
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
        // Whether our own records are published gates what names them: the
        // buddy list's invite and public-key tabs and intent dialog, and
        // every chat window's composer (plan FC3). Each post-login window
        // keeps it in its network store; the login window has nothing to
        // gate.
        SubscriptionEvent::Network(NetworkEvent::OwnRecordsChanged { .. }) => Audience::post_auth(),
        // A co-creation's end closes the creation's status view, in the
        // Create Community dialog of the buddy list or a community window
        // (plan E3.2 S7s.3g).
        SubscriptionEvent::Friend(FriendEvent::CoCreationFinished { .. }) => communities(),
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
            EventScope::Conversation(group) => {
                buddy_list().with_labels(labels::conversation_label(&group))
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
        // Every window applies it, not only the community dispatcher.
        SubscriptionEvent::System(SystemEvent::SkinChanged { .. }) => "skin-event",
        SubscriptionEvent::System(SystemEvent::SkinTrialChanged { .. }) => "skin-trial-event",
        SubscriptionEvent::System(SystemEvent::QuickWipeChanged { .. }) => "quick-wipe-event",
        SubscriptionEvent::Voice(_) => "voice-event",
        // `ChannelMessage` and `Typing` split by context: the community
        // half goes to the community dispatcher, the direct half to the
        // chat handlers.
        SubscriptionEvent::ChannelMessage(
            ChannelMessageEvent::New { .. }
            | ChannelMessageEvent::Edited { .. }
            | ChannelMessageEvent::Deleted { .. }
            | ChannelMessageEvent::ChannelSendStatus { .. }
            | ChannelMessageEvent::ChannelHistoryGap { .. }
            | ChannelMessageEvent::ChannelHistoryGapClosed { .. }
            | ChannelMessageEvent::AttachmentDownloaded { .. }
            | ChannelMessageEvent::ExpressionAssetReady { .. },
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
        | SubscriptionEvent::Friend(_)
        | SubscriptionEvent::GroupDm(_) => "chat-event",
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
                            if let Some(status) = snapshot
                                .status
                                .as_deref()
                                .and_then(|s| serde_json::from_value(s.into()).ok())
                            {
                                crate::tray::show_status(&app, Some(status));
                            }
                        }
                        crate::os_notify::notify(&app, &state, &sequenced.event);
                        crate::services::community::notify_stored_post(
                            &app,
                            &state,
                            &sequenced.event,
                        );
                        crate::windows::follow_event(&app, &state, &sequenced.event);
                        let event = WebviewEvent::Subscription(sequenced.event);
                        if let Some(envelope) = event.envelope(Some(sequenced.cursor)) {
                            state.event_router.deliver(&envelope, &event.audience());
                        }
                    }
                    StreamFrame::InvalidSession => state
                        .event_router
                        .deliver_all(&OutboundEnvelope::invalid_session()),
                    StreamFrame::Resumed(_) => {}
                    // The stream ends here. After a wipe the GUI clears its
                    // webviews and exits (plan E7.1); any other stop is the
                    // GUI's own quit, already under way.
                    StreamFrame::HostStopping { reason } => {
                        if reason == rekindle_types::subscription_events::StopReason::DataWiped {
                            crate::shutdown::exit_after_wipe(&app);
                        }
                        return;
                    }
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
            // Never journaled, so a replay never holds it.
            StreamFrame::HostStopping { .. } => None,
        })
        .collect()
}

/// The session's standing events addressed to `label`, with their cursors
/// (plan E7.1): what a window that just opened is handed before its live
/// events, so a security alert raised at unlock, before the window
/// subscribed, still reaches it.
#[must_use]
pub fn standing_for(host: &rekindle_node::host::Host, label: &str) -> Vec<OutboundEnvelope> {
    host.standing()
        .into_iter()
        .filter_map(|sequenced| {
            let event = WebviewEvent::Subscription(sequenced.event);
            event
                .audience()
                .includes(label)
                .then(|| event.envelope(Some(sequenced.cursor)))
                .flatten()
        })
        .collect()
}

#[cfg(test)]
#[path = "event_dispatch_tests.rs"]
mod tests;

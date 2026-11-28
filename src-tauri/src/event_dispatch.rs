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

use rekindle_types::dto::infoview::InfoviewEvent;
use rekindle_types::key_format::record_key;
use std::sync::Arc;

use rekindle_types::subscription_events::{
    CallEvent, ChannelMessageEvent, EventScope, FriendEvent, MembershipEvent, NetworkEvent,
    SubscriptionEvent, SystemEvent,
};
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tokio::sync::mpsc;

use crate::channels::CommunityEvent;
use crate::event_router::{Audience, OutboundEnvelope};
use crate::state::{AppState, SharedState};
use crate::window_labels::{self as labels, WindowKind};
use crate::windows::SettingsTab;

mod subscription_channel;

use subscription_channel::subscription_channel;

/// Everything the backend sends to a webview.
#[derive(Debug, Clone)]
pub enum WebviewEvent {
    /// The shared vocabulary every frontend observes.
    Subscription(SubscriptionEvent),
    /// Present the rectangular base in a framed desktop window.
    SkinFrameBase { window: String },
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
    /// Show `channel` in the open community window labelled `window` (plan
    /// E7.2 A: the skin's Show Voice on a community voice session).
    CommunityFocusChannel { window: String, channel: String },
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
            Self::SkinFrameBase { .. } => "skin-event",
            Self::Lifecycle { .. } => "lifecycle-event",
            Self::PairingLinkRefused => "pairing-link-refused",
            Self::SettingsSwitchTab(_) => "settings-switch-tab",
            Self::Community(_) => "community-event",
            Self::CommunityFocusChannel { .. } => "community-focus-channel",
        }
    }

    fn payload(&self) -> Result<serde_json::Value, serde_json::Error> {
        match self {
            Self::Subscription(e) => serde_json::to_value(e),
            Self::SkinFrameBase { .. } => Ok(serde_json::json!({
                "system": { "skinChanged": { "skin": null } },
            })),
            Self::Lifecycle { state, at_ms } => serde_json::to_value(LifecyclePayload {
                state: *state,
                at_ms: *at_ms,
            }),
            Self::PairingLinkRefused => Ok(serde_json::Value::Null),
            Self::SettingsSwitchTab(tab) => serde_json::to_value(tab),
            Self::Community(e) => serde_json::to_value(e),
            Self::CommunityFocusChannel { channel, .. } => serde_json::to_value(channel),
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
            Self::SkinFrameBase { window } | Self::CommunityFocusChannel { window, .. } => {
                Audience::labels([window.clone()])
            }
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
        // Framed windows receive this same event from their native
        // allocation controller after applying the matching geometry.
        // Replaying an old Host cursor must not bypass that controller.
        SubscriptionEvent::System(SystemEvent::SkinChanged { .. }) => {
            Audience::kinds(&[
                WindowKind::Login,
                WindowKind::Settings,
                WindowKind::Call,
                WindowKind::Profile,
                WindowKind::SkinEditor,
                WindowKind::SkinPopup,
            ])
            .with_labels([labels::community_label(None)])
        }
        // "Keep this skin?" is asked where skins are chosen: Settings →
        // Appearance renders it from the payload (plan E7.7.2). The quick
        // wipe's state shows in Settings → Privacy, where it is armed (plan
        // E7.1); the shell's own registration follows it in `shell_session`.
        SubscriptionEvent::System(
            SystemEvent::SkinTrialChanged { .. } | SystemEvent::QuickWipeChanged { .. },
        ) => Audience::kinds(&[WindowKind::Settings]),
        // A skin a window could not apply goes to that window, by label,
        // and to Settings, where skins are chosen and a trial ends (plan
        // SK2-T1).
        // Each of these also goes to the one window it names, by label (ADR 0013):
        // - a window's Infoview, asked with its own label (`commands::infoview`);
        // - a skin window's signals (plan SK2-A4 step 3, lead decision 48):
        //   another chat window's set names another friend's capabilities;
        // - a view switch pressed in a popup, addressed by the Host to its
        //   launcher (plan SK2-N4).
        // This arm stays ahead of the general SkinApplyFailed arm below.
        SubscriptionEvent::System(
            SystemEvent::SkinApplyFailed {
                window,
                cause: rekindle_types::dto::skin_library::SkinApplyFailure::WindowTooSmall { .. },
                ..
            }
            | SystemEvent::Infoview(InfoviewEvent::Updated { window, .. })
            | SystemEvent::SkinSignals { window, .. }
            | SystemEvent::SkinShell { window, .. },
        ) => Audience::labels([window.clone()]),
        SubscriptionEvent::System(SystemEvent::SkinApplyFailed { window, .. }) => {
            Audience::labels([window.clone()]).with_kinds(&[WindowKind::Settings])
        }
        SubscriptionEvent::System(
            // The Infoview shows the feed: the buddy list's and each chat
            // window's (plan E7.15, design §2.1).
            SystemEvent::ActivityAppended { .. }
            // The Servers view (plan E7.2 G3): the buddy list's Servers tab
            // (`EngineView::Servers`) and every chat window's skin tree, the
            // windows the Infoview's feed reaches.
            | SystemEvent::ServersViewChanged { .. },
        ) => buddy_list().with_kinds(&[WindowKind::Chat]),
        // One OS notification, one inbox, one ring, one review of an action
        // the backend began: the buddy list owns them. So does the Downloads
        // view (`EngineView::Files`), where its skin's download capabilities
        // fold (plan E7.2 G12).
        SubscriptionEvent::Notification(_)
        | SubscriptionEvent::Transfer(_)
        | SubscriptionEvent::IntentPending(_)
        | SubscriptionEvent::ChannelMessage(ChannelMessageEvent::ConversationFocusRequested {
            ..
        }) => buddy_list(),
        // Our own mute names no call: the buddy list, every call window
        // and every chat window show our mute control (lead decision 58).
        SubscriptionEvent::Call(CallEvent::SelfMuteChanged { .. }) => {
            buddy_list().with_kinds(&[WindowKind::Call, WindowKind::Chat])
        }
        // The call's own window, the buddy list, and the chat windows
        // (each shows the call with its peer).
        SubscriptionEvent::Call(e) => buddy_list()
            .with_labels(labels::call_label_for(e.call_id().unwrap_or_default()))
            .with_kinds(&[WindowKind::Chat]),
        // Whether our own records are published gates what names them: the
        // buddy list's invite and public-key tabs and intent dialog, and
        // every chat window's composer (plan FC3). Each post-login window
        // keeps it in its network store; the login window has nothing to
        // gate.
        SubscriptionEvent::Network(NetworkEvent::OwnRecordsChanged { .. }) => Audience::post_auth(),
        // A join withdrawn after it returned (plan E3.1/JN1) drops the
        // community from every window that may show it: the buddy list's
        // communities, its own community window, and any other post-login
        // window holding it in its store.
        SubscriptionEvent::Membership(MembershipEvent::JoinWithdrawn { .. }) => {
            Audience::post_auth()
        }
        // A route class down or back (plan RV1g): every window shows
        // "Unreachable — retrying since …" or clears it.
        SubscriptionEvent::Network(
            NetworkEvent::OwnRouteDown { .. } | NetworkEvent::OwnRouteUp { .. },
        ) => Audience::kinds(&WindowKind::ALL),
        // A co-creation's end closes the creation's status view, in the
        // Create Community dialog of the buddy list or a community window
        // (plan E3.2 S7s.3g). Our leave, withdrawn join or Forget (plan
        // E3.2): every window holding the community list drops the
        // community, though the community's scope is already torn down.
        SubscriptionEvent::Friend(FriendEvent::CoCreationFinished { .. })
        | SubscriptionEvent::Membership(
            rekindle_types::subscription_events::MembershipEvent::Forgotten { .. },
        ) => communities(),
        // Admission carries a history row. The Host checks the stream's read
        // permission, and its community window owns that history.
        SubscriptionEvent::ChannelMessage(ChannelMessageEvent::PostAdmissionChanged {
            community,
            ..
        }) => match record_key(community) {
            Ok(community) => Audience::labels([labels::community_label(Some(&community))]),
            Err(_) => Audience::default(),
        },
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
pub fn emit_call(app: &AppHandle, event: CallEvent) {
    emit(
        app,
        WebviewEvent::Subscription(SubscriptionEvent::Call(event)),
    );
}

/// Our own microphone mute changed (the mute button or the mute
/// shortcut): a [`CallEvent::SelfMuteChanged`] on the call-event path,
/// which the Host's skin signals fold into `MuteSelf`/`UnmuteSelf` (lead
/// decision 58).
pub fn emit_self_muted(state: &AppState, muted: bool) {
    let timestamp_ms = rekindle_utils::timestamp_ms();
    emit_from_state(
        state,
        WebviewEvent::Subscription(SubscriptionEvent::Call(CallEvent::SelfMuteChanged {
            muted,
            timestamp_ms,
        })),
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

#[cfg(test)]
#[path = "event_dispatch_tests.rs"]
mod tests;

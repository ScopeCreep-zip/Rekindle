//! A skin window's state signals (plan SK2-A4 step 3, design §2.5): the
//! Host folds its own event stream (typing, calls, voice activity, the
//! network, notifications) and the shell's window focus into per-window
//! inputs, computes each window's [`SkinSignalSet`] from them, the
//! window's selection ([`super::actions::Selections`]) and the selected
//! friend's state in the session, and publishes
//! `SystemEvent::SkinSignals` as the session state `skin-signals:<label>`
//! whenever a window's set changes. Windows are keyed by label (lead
//! decision 48): two chat windows have two selections and two sets.
//!
//! A capability is `cannot` only for a real state condition, as Xfire's
//! were (nothing selected, the friend offline, no joinable server, no
//! call): never because a feature is missing.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use rekindle_types::dto::skin_action::Capability;
use rekindle_types::dto::skin_signals::{SkinSignalSet, EVERY_CAPABILITY};
use rekindle_types::friend::{FriendState, FriendshipState, UserStatus};
use rekindle_types::subscription_events::{
    CallEvent, NetworkEvent, StreamFrame, SubscriptionEvent, SystemEvent, TypingContext,
    TypingEvent, VoiceEvent,
};

use crate::daemon::dispatch::DaemonContext;

/// A call the Host has seen on its stream, by call id.
#[derive(Debug, Clone)]
struct Call {
    /// The 1:1 peer; `None` for a group call.
    peer: Option<String>,
    ringing: bool,
    outgoing: bool,
}

/// What the signals are computed from, besides the selection and the session.
#[derive(Debug, Default)]
pub(crate) struct Inputs {
    /// Every skin window the shell has told us of, by label.
    windows: BTreeSet<String>,
    focused: BTreeSet<String>,
    typing: BTreeSet<String>,
    calls: BTreeMap<String, Call>,
    speaking: BTreeSet<String>,
    /// Our microphone mute in the call (the desktop call engine's until
    /// E4.1); cleared when no call is left.
    self_muted: bool,
    disconnected: bool,
    unseen_notification: BTreeSet<String>,
}

impl Inputs {
    /// The shell's window focus. A window that gains focus has seen its
    /// notifications.
    pub(crate) fn set_focus(&mut self, window: &str, focused: bool) {
        self.windows.insert(window.to_owned());
        if focused {
            self.focused.insert(window.to_owned());
            self.unseen_notification.remove(window);
        } else {
            self.focused.remove(window);
        }
    }

    /// A window the shell opened (its focus may come later).
    pub(crate) fn open(&mut self, window: &str) {
        self.windows.insert(window.to_owned());
    }

    /// A window closed: it carries no signals any more.
    pub(crate) fn close(&mut self, window: &str) {
        self.windows.remove(window);
        self.focused.remove(window);
        self.unseen_notification.remove(window);
    }

    /// Our microphone mute in the call.
    pub(crate) fn set_self_muted(&mut self, muted: bool) {
        self.self_muted = muted;
    }

    fn in_call(&self) -> bool {
        self.calls.values().any(|c| !c.ringing)
    }

    /// Fold one event; `false` when it cannot change any window's set.
    pub(crate) fn fold(&mut self, event: &SubscriptionEvent) -> bool {
        match event {
            SubscriptionEvent::Typing(TypingEvent::Started {
                context: TypingContext::Dm { peer_key },
                ..
            }) => self.typing.insert(peer_key.clone()),
            SubscriptionEvent::Typing(TypingEvent::Stopped {
                context: TypingContext::Dm { peer_key },
                ..
            }) => self.typing.remove(peer_key),
            SubscriptionEvent::Call(call) => self.fold_call(call),
            SubscriptionEvent::Voice(VoiceEvent::SpeakingChanged {
                pseudonym,
                speaking,
                ..
            }) => {
                if *speaking {
                    self.speaking.insert(pseudonym.clone())
                } else {
                    self.speaking.remove(pseudonym)
                }
            }
            SubscriptionEvent::Network(NetworkEvent::AttachmentChanged { is_attached, .. }) => {
                self.disconnected = !is_attached;
                true
            }
            SubscriptionEvent::Notification(_) => {
                self.unseen_notification = self
                    .windows
                    .iter()
                    .filter(|w| !self.focused.contains(*w))
                    .cloned()
                    .collect();
                true
            }
            SubscriptionEvent::Presence(_) | SubscriptionEvent::Friend(_) => true,
            _ => false,
        }
    }

    fn fold_call(&mut self, event: &CallEvent) -> bool {
        match event {
            CallEvent::Incoming {
                call_id,
                from,
                is_group,
                ..
            } => {
                let peer = (!is_group).then(|| from.clone());
                let call = Call {
                    peer,
                    ringing: true,
                    outgoing: false,
                };
                self.calls.insert(call_id.clone(), call);
            }
            CallEvent::Started {
                call_id, peer_key, ..
            } => {
                let call = Call {
                    peer: Some(peer_key.clone()),
                    ringing: true,
                    outgoing: true,
                };
                self.calls.insert(call_id.clone(), call);
            }
            CallEvent::Connected { call_id, direct } => {
                let call = self.calls.entry(call_id.clone()).or_insert(Call {
                    peer: None,
                    ringing: false,
                    outgoing: false,
                });
                call.ringing = false;
                if let Some(direct) = direct {
                    call.peer = Some(direct.peer_key.clone());
                }
            }
            CallEvent::Declined { call_id, .. }
            | CallEvent::Missed { call_id, .. }
            | CallEvent::TimedOut { call_id }
            | CallEvent::Ended { call_id, .. } => {
                self.calls.remove(call_id);
                if self.calls.is_empty() {
                    self.self_muted = false;
                }
            }
            _ => return false,
        }
        true
    }

    fn call_with(&self, peer: &str) -> Option<&Call> {
        self.calls
            .values()
            .find(|c| c.peer.as_deref() == Some(peer))
    }
}

/// Whether `capability` holds: `selected` is the selected key and the
/// friend it names, if any. Each `false` is a state condition Xfire's
/// `OnAppCannot<X>` followed.
fn allows(
    capability: Capability,
    inputs: &Inputs,
    selected: Option<(&str, Option<&FriendState>)>,
) -> bool {
    let key = selected.map(|(k, _)| k);
    let friend = selected.and_then(|(_, f)| f);
    let accepted = friend.is_some_and(|f| f.friendship_state == FriendshipState::Accepted);
    let online = accepted && friend.is_some_and(|f| f.status != UserStatus::Offline);
    let call = key.and_then(|k| inputs.call_with(k));
    let server = friend
        .and_then(|f| f.game_info.as_ref())
        .is_some_and(|g| g.server_address.is_some());
    match capability {
        // A key that is not a friend yet.
        Capability::AddFriend => selected.is_some() && friend.is_none(),
        Capability::Chat => accepted,
        Capability::Profile | Capability::Delete => selected.is_some(),
        // An online friend in a game with a server we can join.
        Capability::Join | Capability::JoinGameLobby => online && server,
        Capability::VoiceChat | Capability::HostVoiceChat => online && call.is_none(),
        Capability::LeaveVoiceChat => call.is_some_and(|c| !c.ringing),
        Capability::CancelVoiceChat => call.is_some_and(|c| c.ringing && c.outgoing),
        Capability::MuteSelf => inputs.in_call() && !inputs.self_muted,
        Capability::UnmuteSelf => inputs.in_call() && inputs.self_muted,
        // No state gates these: each is always available.
        Capability::AddServer
        | Capability::RefreshServer
        | Capability::StartDownload
        | Capability::HostGameLobby
        | Capability::CreateRoom
        | Capability::EnterRoom => true,
        // These act on something selected in the window.
        Capability::DeleteSelected | Capability::OpenSelected | Capability::UploadSelected => {
            selected.is_some()
        }
        // These act on a download or a lobby in progress: there is none
        // until one starts.
        Capability::StopDownload
        | Capability::ExecuteDownload
        | Capability::LaunchGameLobby
        | Capability::LeaveGameLobby => false,
    }
}

/// One window's set: `selected` is the selected key and its friend, if any.
pub(crate) fn compute(
    window: &str,
    inputs: &Inputs,
    selected: Option<(&str, Option<&FriendState>)>,
) -> SkinSignalSet {
    let mut set = SkinSignalSet {
        focused: inputs.focused.contains(window),
        disconnected: inputs.disconnected,
        new_notification: inputs.unseen_notification.contains(window),
        ..SkinSignalSet::default()
    };
    for capability in EVERY_CAPABILITY {
        if allows(capability, inputs, selected) {
            set.can.insert(capability);
        } else {
            set.cannot.insert(capability);
        }
    }
    if let Some((key, _)) = selected {
        set.user_typing = inputs.typing.contains(key);
        set.voice_ringing = inputs.call_with(key).is_some_and(|c| c.ringing);
        set.talking = inputs.speaking.contains(key);
    }
    set
}

/// Every known window's set, by label: `friend` looks a key up in the
/// session.
pub(crate) fn window_sets(
    inputs: &Inputs,
    selections: &super::actions::Selections,
    friend: impl Fn(&str) -> Option<FriendState>,
) -> BTreeMap<String, SkinSignalSet> {
    let mut windows: BTreeSet<String> = inputs.windows.clone();
    windows.extend(selections.windows());
    windows
        .into_iter()
        .map(|window| {
            let key = selections.friend(&window);
            let f = key.as_deref().and_then(&friend);
            let set = compute(&window, inputs, key.as_deref().map(|k| (k, f.as_ref())));
            (window, set)
        })
        .collect()
}

/// The Host's signal state: the inputs, the sets last published, and the
/// worker's wake-up for an input that is not on the stream.
#[derive(Debug, Default)]
pub(crate) struct SignalState {
    inputs: parking_lot::Mutex<Inputs>,
    last: parking_lot::Mutex<BTreeMap<String, SkinSignalSet>>,
    poke: tokio::sync::Notify,
}

impl SignalState {
    /// A selection or the focus changed: recompute.
    pub(crate) fn poke(&self) {
        self.poke.notify_one();
    }

    /// The shell's window focus (`Host::skin_window_focused`).
    pub(crate) fn set_focus(&self, window: &str, focused: bool) {
        self.inputs.lock().set_focus(window, focused);
        self.poke();
    }
}

/// Each window's set that differs from the one last published.
pub(crate) fn changed(ctx: &DaemonContext) -> Vec<(String, SkinSignalSet)> {
    let sets = {
        let session = ctx.session.read();
        let inputs = ctx.skin_signals.inputs.lock();
        window_sets(&inputs, &ctx.skin_selection, |k| {
            session.as_ref().and_then(|s| s.friends.get(k).cloned())
        })
    };
    let mut last = ctx.skin_signals.last.lock();
    last.retain(|w, _| sets.contains_key(w));
    let mut out = Vec::new();
    for (window, set) in sets {
        if last.get(&window) != Some(&set) {
            last.insert(window.clone(), set.clone());
            out.push((window, set));
        }
    }
    out
}

/// Publish every changed set.
pub(crate) fn publish_changes(ctx: &DaemonContext) {
    for (window, set) in changed(ctx) {
        ctx.journal.publish_state(
            &format!("skin-signals:{window}"),
            SubscriptionEvent::System(SystemEvent::SkinSignals { window, set }),
        );
    }
}

/// The worker: fold the Host's stream and the pokes until shutdown.
pub(crate) async fn run_worker(ctx: Arc<DaemonContext>) {
    let mut stream = ctx.journal.subscribe(None);
    publish_changes(&ctx);
    loop {
        tokio::select! {
            () = ctx.shutdown.requested() => return,
            frame = stream.next() => match frame {
                Some(StreamFrame::Event(s)) => {
                    if !ctx.skin_signals.inputs.lock().fold(&s.event) {
                        continue;
                    }
                }
                Some(_) => {}
                None => return,
            },
            () = ctx.skin_signals.poke.notified() => {}
        }
        publish_changes(&ctx);
    }
}

impl crate::host::Host {
    /// The shell's window focus for the skin window labelled `window`
    /// (the Tauri window-focus event): it drives the window's
    /// `OnActivate`/`OnDeactivate`.
    pub fn skin_window_focused(&self, window: &str, focused: bool) {
        self.ctx.skin_signals.set_focus(window, focused);
    }

    /// The shell opened a skin window; `selection` is what it shows from
    /// the start (a chat window's own peer, lead decision 48).
    pub fn skin_window_opened(
        &self,
        window: &str,
        selection: Option<rekindle_types::dto::skin_run::SkinSelection>,
    ) {
        self.ctx.skin_signals.inputs.lock().open(window);
        self.ctx.skin_selection.set(window, selection);
        self.ctx.skin_signals.poke();
    }

    /// A skin window closed: its selection and signals go with it.
    pub fn skin_window_closed(&self, window: &str) {
        self.ctx.skin_signals.inputs.lock().close(window);
        self.ctx.skin_selection.set(window, None);
        self.ctx.journal.clear_state(&format!("skin-signals:{window}"));
        self.ctx.skin_signals.poke();
    }

    /// Our microphone mute in the call, from the desktop call engine
    /// (until E4.1 moves calls into the Host): `OnAppCanMuteSelf` /
    /// `OnAppCanUnmuteSelf`.
    pub fn skin_self_muted(&self, muted: bool) {
        self.ctx.skin_signals.inputs.lock().set_self_muted(muted);
        self.ctx.skin_signals.poke();
    }
}

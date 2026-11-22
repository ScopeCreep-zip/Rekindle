//! A skin window's state signals (plan SK2-A4 step 3, design §2.5): the
//! Host folds its own event stream (typing, calls, voice activity, the
//! network, notifications) and the shell's window focus into per-window
//! inputs, computes each window's [`SkinSignalSet`] from them, the
//! window's selection ([`super::actions::Selections`]) and the selected
//! friend's state in the session, and publishes
//! `SystemEvent::SkinSignals` as the session state `skin-signals:<window>`
//! whenever a window's set changes.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use rekindle_types::dto::skin_action::Capability;
use rekindle_types::dto::skin_layout::SkinWindow;
use rekindle_types::dto::skin_signals::{SkinSignalSet, EVERY_CAPABILITY};
use rekindle_types::friend::{FriendState, FriendshipState, UserStatus};
use rekindle_types::subscription_events::{
    CallEvent, NetworkEvent, StreamFrame, SubscriptionEvent, SystemEvent, TypingContext,
    TypingEvent, VoiceEvent,
};

use crate::daemon::dispatch::DaemonContext;

/// The skin windows that carry signals.
const WINDOWS: [SkinWindow; 2] = [SkinWindow::Main, SkinWindow::Chat];

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
    focused: BTreeSet<SkinWindow>,
    typing: BTreeSet<String>,
    calls: BTreeMap<String, Call>,
    speaking: BTreeSet<String>,
    disconnected: bool,
    unseen_notification: BTreeSet<SkinWindow>,
}

impl Inputs {
    /// The shell's window focus. A window that gains focus has seen its
    /// notifications.
    pub(crate) fn set_focus(&mut self, window: SkinWindow, focused: bool) {
        if focused {
            self.focused.insert(window);
            self.unseen_notification.remove(&window);
        } else {
            self.focused.remove(&window);
        }
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
                self.unseen_notification = WINDOWS
                    .into_iter()
                    .filter(|w| !self.focused.contains(w))
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

/// Whether the selection (a key, and the friend it names if any) allows
/// `capability`.
fn allows(
    capability: Capability,
    inputs: &Inputs,
    key: &str,
    friend: Option<&FriendState>,
) -> bool {
    let accepted = friend.is_some_and(|f| f.friendship_state == FriendshipState::Accepted);
    let online = accepted && friend.is_some_and(|f| f.status != UserStatus::Offline);
    let call = inputs.call_with(key);
    match capability {
        Capability::AddFriend => friend.is_none(),
        Capability::Chat => accepted,
        Capability::Profile => true,
        Capability::Join => {
            online
                && friend
                    .and_then(|f| f.game_info.as_ref())
                    .is_some_and(|g| g.server_address.is_some())
        }
        Capability::VoiceChat | Capability::HostVoiceChat => online && call.is_none(),
        Capability::LeaveVoiceChat => call.is_some_and(|c| !c.ringing),
        Capability::CancelVoiceChat => call.is_some_and(|c| c.ringing && c.outgoing),
        // Self-mute is the desktop call engine's state until E4.1 moves
        // calls into the Host; servers, downloads, lobbies, rooms and the
        // selected-item operations are planned feature gaps (plan SK2-A4).
        Capability::MuteSelf
        | Capability::UnmuteSelf
        | Capability::AddServer
        | Capability::RefreshServer
        | Capability::StartDownload
        | Capability::StopDownload
        | Capability::ExecuteDownload
        | Capability::HostGameLobby
        | Capability::JoinGameLobby
        | Capability::LaunchGameLobby
        | Capability::LeaveGameLobby
        | Capability::CreateRoom
        | Capability::EnterRoom
        | Capability::Delete
        | Capability::DeleteSelected
        | Capability::OpenSelected
        | Capability::UploadSelected => false,
    }
}

/// One window's set: `selected` is the selected key and its friend, if any.
pub(crate) fn compute(
    window: SkinWindow,
    inputs: &Inputs,
    selected: Option<(&str, Option<&FriendState>)>,
) -> SkinSignalSet {
    let mut set = SkinSignalSet {
        focused: inputs.focused.contains(&window),
        disconnected: inputs.disconnected,
        new_notification: inputs.unseen_notification.contains(&window),
        ..SkinSignalSet::default()
    };
    for capability in EVERY_CAPABILITY {
        let can = selected.is_some_and(|(key, friend)| allows(capability, inputs, key, friend));
        if can {
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

/// The Host's signal state: the inputs, the sets last published, and the
/// worker's wake-up for an input that is not on the stream.
#[derive(Debug, Default)]
pub(crate) struct SignalState {
    inputs: parking_lot::Mutex<Inputs>,
    last: parking_lot::Mutex<BTreeMap<SkinWindow, SkinSignalSet>>,
    poke: tokio::sync::Notify,
}

impl SignalState {
    /// A selection or the focus changed: recompute.
    pub(crate) fn poke(&self) {
        self.poke.notify_one();
    }

    /// The shell's window focus (`Host::skin_window_focused`).
    pub(crate) fn set_focus(&self, window: SkinWindow, focused: bool) {
        self.inputs.lock().set_focus(window, focused);
        self.poke();
    }
}

/// Each window's set that differs from the one last published.
pub(crate) fn changed(ctx: &DaemonContext) -> Vec<(SkinWindow, SkinSignalSet)> {
    let mut out = Vec::new();
    for window in WINDOWS {
        let key = ctx.skin_selection.friend(window);
        let friend = key.as_deref().and_then(|k| {
            ctx.session
                .read()
                .as_ref()
                .and_then(|s| s.friends.get(k).cloned())
        });
        let set = {
            let inputs = ctx.skin_signals.inputs.lock();
            compute(
                window,
                &inputs,
                key.as_deref().map(|k| (k, friend.as_ref())),
            )
        };
        let mut last = ctx.skin_signals.last.lock();
        if last.get(&window) != Some(&set) {
            last.insert(window, set.clone());
            out.push((window, set));
        }
    }
    out
}

/// Publish every changed set.
pub(crate) fn publish_changes(ctx: &DaemonContext) {
    for (window, set) in changed(ctx) {
        ctx.journal.publish_state(
            &format!("skin-signals:{window:?}"),
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
    /// The shell's window focus for a skin window (the Tauri window-focus
    /// event): it drives the window's `OnActivate`/`OnDeactivate`.
    pub fn skin_window_focused(&self, window: SkinWindow, focused: bool) {
        self.ctx.skin_signals.set_focus(window, focused);
    }
}

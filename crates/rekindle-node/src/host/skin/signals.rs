//! A skin window's state signals (plan SK2-A4 step 3, design §2.5): the
//! Host folds its own event stream (typing, calls, voice activity, the
//! network) and the shell's window focus into per-window inputs. It
//! computes each window's [`SkinSignalSet`] from those inputs, the
//! journal's unread alert state, the window's selection
//! ([`super::actions::Selections`]) and the selected friend's state, and publishes
//! `SystemEvent::SkinSignals` as the session state `skin-signals:<label>`
//! whenever a window's set changes. Windows are keyed by label (lead
//! decision 48): two chat windows have two selections and two sets.
//!
//! A capability is `cannot` only for a real state condition, as Xfire's
//! were (nothing selected, the friend offline, no joinable server, no
//! call): never because a feature is missing.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use rekindle_types::dto::skin_run::{CallTarget, SkinSelection};
use rekindle_types::dto::skin_signals::{
    DisableCause, DisableSignal, SkinSignalSet, EVERY_CAPABILITY,
};
use rekindle_types::friend::FriendState;
use rekindle_types::subscription_events::{
    CallEvent, NetworkEvent, NotificationEvent, StreamFrame, SubscriptionEvent, SystemEvent,
    TypingContext, TypingEvent, VoiceEvent, VoiceScope,
};

use super::capability::{refusal, Selected, SkinWorld};
pub(crate) use super::world::load_world;
use crate::daemon::dispatch::DaemonContext;

/// A call the Host has seen on its stream, by call id.
#[derive(Debug, Clone)]
pub(crate) struct Call {
    /// The 1:1 peer; `None` for a group call.
    peer: Option<String>,
    /// The group DM a group call belongs to, from the `groupId` on
    /// `CallEvent::Incoming`/`Connected` (`GroupCallOffer.group_id`,
    /// roster-checked by `rekindle_calls::signaling::check_group_scope`).
    group: Option<String>,
    pub(crate) ringing: bool,
    pub(crate) outgoing: bool,
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
    /// Our microphone mute in the call, folded from
    /// `CallEvent::SelfMuteChanged` on the stream (lead decision 58);
    /// cleared when no call is left.
    self_muted: bool,
    disconnected: bool,
    /// Our voice session, from `VoiceEvent::LocalJoined`; cleared by the
    /// `VoiceEvent::Left` that names us (`own`), as our own leave does
    /// (`rekindle_voice::session::leave_voice`).
    voice: Option<VoiceScope>,
    /// Our identity key, which tells our own `Left` from a peer's.
    own: Option<String>,
}

impl Inputs {
    /// The shell's window focus, independent of unread alerts.
    pub(crate) fn set_focus(&mut self, window: &str, focused: bool) {
        self.windows.insert(window.to_owned());
        if focused {
            self.focused.insert(window.to_owned());
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
    }

    pub(crate) fn in_call(&self) -> bool {
        self.calls.values().any(|c| !c.ringing)
    }

    pub(crate) fn self_muted(&self) -> bool {
        self.self_muted
    }

    /// Our voice session (a community window's Call, Leave and Mute).
    pub(crate) fn voice(&self) -> Option<&VoiceScope> {
        self.voice.as_ref()
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
            SubscriptionEvent::Voice(VoiceEvent::LocalJoined { scope, .. }) => {
                self.voice = Some(scope.clone());
                true
            }
            SubscriptionEvent::Voice(VoiceEvent::Left { scope, pseudonym }) => {
                let ours = self.own.as_deref() == Some(pseudonym.as_str());
                if ours && self.voice.as_ref() == Some(scope) {
                    self.voice = None;
                }
                false
            }
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
            // The signal pass reads alert state from the journal, a
            // transfer's state from `transfers`, and the own-records
            // stage from `FriendStores::own_records`.
            SubscriptionEvent::Notification(NotificationEvent::SystemAlert { .. })
            | SubscriptionEvent::Network(NetworkEvent::OwnRecordsChanged { .. })
            | SubscriptionEvent::Presence(_)
            | SubscriptionEvent::Friend(_)
            | SubscriptionEvent::Transfer(_) => true,
            _ => false,
        }
    }

    fn fold_call(&mut self, event: &CallEvent) -> bool {
        match event {
            CallEvent::Incoming {
                call_id,
                from,
                is_group,
                group_id,
                ..
            } => {
                let peer = (!is_group).then(|| from.clone());
                let call = Call {
                    peer,
                    group: group_id.clone(),
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
                    group: None,
                    ringing: true,
                    outgoing: true,
                };
                self.calls.insert(call_id.clone(), call);
            }
            CallEvent::GroupStarted {
                call_id, group_id, ..
            } => {
                let call = Call {
                    peer: None,
                    group: group_id.clone(),
                    ringing: true,
                    outgoing: true,
                };
                self.calls.insert(call_id.clone(), call);
            }
            CallEvent::Connected {
                call_id,
                direct,
                group_id,
            } => {
                let call = self.calls.entry(call_id.clone()).or_insert(Call {
                    peer: None,
                    group: None,
                    ringing: false,
                    outgoing: false,
                });
                call.ringing = false;
                if group_id.is_some() {
                    call.group.clone_from(group_id);
                }
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
            CallEvent::SelfMuteChanged { muted, .. } => {
                if self.self_muted == *muted {
                    return false;
                }
                self.self_muted = *muted;
            }
            _ => return false,
        }
        true
    }

    /// Our identity key (the session's owner), to tell our own voice
    /// `Left` from a peer's.
    pub(crate) fn set_own(&mut self, own: Option<String>) {
        self.own = own;
    }

    /// `ShowVoiceWindow`'s window: the active call (a connected one before
    /// a ringing one), else our community voice session. A DM voice scope
    /// is a call, whose window the call branch names.
    pub(crate) fn show_call(&self) -> Option<CallTarget> {
        let call = self
            .calls
            .iter()
            .find(|(_, c)| !c.ringing)
            .or_else(|| self.calls.iter().next());
        if let Some((call_id, _)) = call {
            return Some(CallTarget::Call {
                call_id: call_id.clone(),
            });
        }
        match self.voice.as_ref()? {
            VoiceScope::Community { community, channel } => Some(CallTarget::Voice {
                community: community.clone(),
                channel: channel.clone(),
            }),
            VoiceScope::Dm { .. } => None,
        }
    }

    /// The id of the incoming 1:1 call from `peer` that is ringing, the
    /// call Xfire's `AppVoiceChat` answers.
    pub(crate) fn ringing_from(&self, peer: &str) -> Option<String> {
        self.calls
            .iter()
            .find(|(_, c)| c.peer.as_deref() == Some(peer) && c.ringing && !c.outgoing)
            .map(|(call_id, _)| call_id.clone())
    }

    /// The call of the group DM `group_id`: the one whose signaling
    /// carries that group id (never matched by members, so two groups of
    /// the same people each see only their own call).
    pub(crate) fn group_call(&self, group_id: &str) -> Option<(&String, &Call)> {
        self.calls
            .iter()
            .find(|(_, c)| c.peer.is_none() && c.group.as_deref() == Some(group_id))
    }

    /// The id of the group's call in progress that we are not in (it is
    /// ringing here), the call Xfire's group `AppGroupVoiceChat` joins.
    pub(crate) fn joinable_group_call(&self, group_id: &str) -> Option<String> {
        self.group_call(group_id)
            .filter(|(_, c)| c.ringing && !c.outgoing)
            .map(|(call_id, _)| call_id.clone())
    }

    /// The id of the group's call that is ours to leave: one we are in,
    /// or our own that still rings out (its cancel), the call Xfire's
    /// group `AppLeaveGroupVoiceChat` leaves.
    pub(crate) fn our_group_call(&self, group_id: &str) -> Option<String> {
        self.group_call(group_id)
            .filter(|(_, c)| !c.ringing || c.outgoing)
            .map(|(call_id, _)| call_id.clone())
    }

    /// The id of the 1:1 call with `peer` that we are in (connected), the
    /// call a chat window's `AppLeaveVoiceChat` leaves.
    pub(crate) fn call_in_with(&self, peer: &str) -> Option<String> {
        self.calls
            .iter()
            .find(|(_, c)| c.peer.as_deref() == Some(peer) && !c.ringing)
            .map(|(call_id, _)| call_id.clone())
    }

    pub(crate) fn call_with(&self, peer: &str) -> Option<&Call> {
        self.calls
            .values()
            .find(|c| c.peer.as_deref() == Some(peer))
    }
}

/// One window's set.
pub(crate) fn compute(
    window: &str,
    inputs: &Inputs,
    world: &SkinWorld,
    selected: Selected<'_>,
) -> SkinSignalSet {
    let mut set = SkinSignalSet {
        focused: inputs.focused.contains(window),
        disconnected: inputs.disconnected,
        new_notification: world.unread_alert,
        ..SkinSignalSet::default()
    };
    for capability in EVERY_CAPABILITY {
        match refusal(capability, inputs, world, selected) {
            None => {
                set.can.insert(capability);
            }
            Some(why) => {
                set.cannot.insert(capability);
                set.disabled_reasons
                    .insert(DisableCause::Capability(capability), why);
            }
        }
    }
    if let Some(key) = selected.key() {
        set.user_typing = inputs.typing.contains(key);
        set.voice_ringing = inputs.call_with(key).is_some_and(|c| c.ringing);
        set.talking = inputs.speaking.contains(key);
    }
    let active = [
        (set.disconnected, DisableSignal::Disconnected),
        (set.user_typing, DisableSignal::ActiveUserTyping),
        (set.voice_ringing, DisableSignal::VoiceRinging),
        (set.talking, DisableSignal::UserStartedTalking),
        (set.new_notification, DisableSignal::NewNotification),
    ];
    for (on, signal) in active {
        if on {
            set.disabled_reasons.insert(
                DisableCause::Signal(signal),
                signal_reason(signal).to_owned(),
            );
        }
    }
    set
}

/// The words for a node a pack disables on an active signal (plan E7.7
/// CR1): what the window shows over it.
pub(crate) const fn signal_reason(signal: DisableSignal) -> &'static str {
    match signal {
        DisableSignal::Disconnected => "You're offline. Rekindle reconnects on its own.",
        DisableSignal::ActiveUserTyping => "Your friend is typing.",
        DisableSignal::VoiceRinging => "A call with your friend is ringing.",
        DisableSignal::UserStartedTalking => "Your friend is talking.",
        DisableSignal::NewNotification => "You have a new notification.",
    }
}

/// Every known window's set, by label, judged on the selection the window
/// shows (a popup's is its launcher's, `Launchers::selection_window`, plan
/// E7.7 HSC1): `friend` looks a key up in the session.
pub(crate) fn window_sets(
    inputs: &Inputs,
    selections: &super::actions::Selections,
    launchers: &super::actions::Launchers,
    world: &SkinWorld,
    friend: impl Fn(&str) -> Option<FriendState>,
) -> BTreeMap<String, SkinSignalSet> {
    let mut windows: BTreeSet<String> = inputs.windows.clone();
    windows.extend(selections.windows());
    windows
        .into_iter()
        .map(|window| {
            let selection = selections.get(&launchers.selection_window(&window));
            let f = match &selection {
                Some(SkinSelection::Friend { public_key }) => friend(public_key),
                _ => None,
            };
            let selected = Selected::of(selection.as_ref(), f.as_ref());
            let set = compute(&window, inputs, world, selected);
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

    /// `ShowVoiceWindow`'s window ([`Inputs::show_call`]).
    pub(crate) fn show_call(&self) -> Option<CallTarget> {
        self.inputs.lock().show_call()
    }

    /// `VoiceChat`'s call ([`Inputs::ringing_from`]).
    pub(crate) fn ringing_from(&self, peer: &str) -> Option<String> {
        self.inputs.lock().ringing_from(peer)
    }

    /// A group window's `GroupVoiceChat` call
    /// ([`Inputs::joinable_group_call`]).
    pub(crate) fn joinable_group_call(&self, group_id: &str) -> Option<String> {
        self.inputs.lock().joinable_group_call(group_id)
    }

    /// A group window's leave or cancel ([`Inputs::our_group_call`]).
    pub(crate) fn our_group_call(&self, group_id: &str) -> Option<String> {
        self.inputs.lock().our_group_call(group_id)
    }

    /// A chat window's leave ([`Inputs::call_in_with`]).
    pub(crate) fn call_in_with(&self, peer: &str) -> Option<String> {
        self.inputs.lock().call_in_with(peer)
    }
}

/// Each window's set that differs from the one last published.
pub(crate) fn changed(ctx: &DaemonContext, world: &SkinWorld) -> Vec<(String, SkinSignalSet)> {
    let sets = {
        let session = ctx.session.read();
        let inputs = ctx.skin.signals.inputs.lock();
        window_sets(
            &inputs,
            &ctx.skin.selection,
            &ctx.skin.launchers,
            world,
            |k| session.as_ref().and_then(|s| s.friends.get(k).cloned()),
        )
    };
    let mut last = ctx.skin.signals.last.lock();
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
async fn publish_changes(ctx: &DaemonContext) {
    let mut world = load_world(ctx).await;
    world.unread_alert = ctx.journal.has_unread_alert();
    for (window, set) in changed(ctx, &world) {
        ctx.journal.publish_state(
            &format!("skin-signals:{window}"),
            SubscriptionEvent::System(SystemEvent::SkinSignals { window, set }),
        );
    }
}

/// The worker: fold the Host's stream and the pokes until shutdown.
pub(crate) async fn run_worker(ctx: Arc<DaemonContext>) {
    let mut stream = ctx.journal.subscribe(None);
    publish_changes(&ctx).await;
    loop {
        tokio::select! {
            () = ctx.shutdown.requested() => return,
            frame = stream.next() => match frame {
                Some(StreamFrame::Event(s)) => {
                    let own = ctx.require_owner().ok();
                    let mut inputs = ctx.skin.signals.inputs.lock();
                    inputs.set_own(own);
                    if !inputs.fold(&s.event) {
                        continue;
                    }
                }
                Some(_) => {}
                None => return,
            },
            () = ctx.skin.signals.poke.notified() => {}
        }
        publish_changes(&ctx).await;
    }
}

impl crate::host::Host {
    /// The shell's window focus for the skin window labelled `window`
    /// (the Tauri window-focus event): it drives the window's
    /// `OnActivate`/`OnDeactivate`.
    pub fn skin_window_focused(&self, window: &str, focused: bool) {
        self.ctx.skin.signals.set_focus(window, focused);
    }

    /// The shell opened a skin window; `launcher` is the window that
    /// launched it when it is a pack popup (its view switches are the
    /// launcher's, plan SK2-N4), and `selection` what it shows from the
    /// start (a chat window's own peer, lead decision 48; a community
    /// window's `SkinSelection::Community`, plan E7.7 HSC1).
    pub fn skin_window_opened(
        &self,
        window: &str,
        launcher: Option<&str>,
        selection: Option<rekindle_types::dto::skin_run::SkinSelection>,
    ) {
        self.ctx.skin.launchers.set(window, launcher);
        self.ctx.skin.signals.inputs.lock().open(window);
        self.ctx.skin.selection.set(window, selection);
        self.ctx.skin.signals.poke();
        self.ctx.infoview.selection_changed(window);
    }

    /// A skin window closed: its selection and signals go with it.
    pub fn skin_window_closed(&self, window: &str) {
        self.ctx.skin.signals.inputs.lock().close(window);
        self.ctx.skin.selection.set(window, None);
        self.ctx.skin.launchers.set(window, None);
        self.ctx.infoview.close(window);
        self.ctx
            .journal
            .clear_state(&format!("skin-signals:{window}"));
        self.ctx.skin.signals.poke();
    }
}

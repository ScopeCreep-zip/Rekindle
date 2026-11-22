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

use rekindle_types::dto::skin_run::SkinSelection;
use rekindle_types::dto::skin_signals::{SkinSignalSet, EVERY_CAPABILITY};
use rekindle_types::friend::{FriendState, FriendshipState};
use rekindle_types::subscription_events::{
    CallEvent, NetworkEvent, StreamFrame, SubscriptionEvent, SystemEvent, TypingContext,
    TypingEvent, VoiceEvent,
};

use super::capability::{allows, Group, Selected, SkinWorld, Transfer};
use crate::daemon::dispatch::DaemonContext;

/// A call the Host has seen on its stream, by call id.
#[derive(Debug, Clone)]
pub(crate) struct Call {
    /// The 1:1 peer; `None` for a group call.
    peer: Option<String>,
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
    /// Our microphone mute in the call (the desktop call engine's until
    /// E4.1); cleared when no call is left.
    self_muted: bool,
    disconnected: bool,
    unseen_notification: BTreeSet<String>,
    /// The downloads the Host ran, by attachment id.
    transfers: BTreeMap<String, Transfer>,
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

    pub(crate) fn in_call(&self) -> bool {
        self.calls.values().any(|c| !c.ringing)
    }

    pub(crate) fn self_muted(&self) -> bool {
        self.self_muted
    }

    /// A download's state, by attachment id.
    pub(crate) fn transfer(&self, attachment_id: &str) -> Option<&Transfer> {
        self.transfers.get(attachment_id)
    }

    pub(crate) fn set_transfer(&mut self, attachment_id: &str, transfer: Transfer) {
        self.transfers.insert(attachment_id.to_owned(), transfer);
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
        new_notification: inputs.unseen_notification.contains(window),
        ..SkinSignalSet::default()
    };
    for capability in EVERY_CAPABILITY {
        if allows(capability, inputs, world, selected) {
            set.can.insert(capability);
        } else {
            set.cannot.insert(capability);
        }
    }
    if let Some(key) = selected.key() {
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
    world: &SkinWorld,
    friend: impl Fn(&str) -> Option<FriendState>,
) -> BTreeMap<String, SkinSignalSet> {
    let mut windows: BTreeSet<String> = inputs.windows.clone();
    windows.extend(selections.windows());
    windows
        .into_iter()
        .map(|window| {
            let selection = selections.get(&window);
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

/// The state the capabilities read besides the selection and the stream:
/// the session's friends and communities, our shown game, and our
/// favourite servers, groups and group invites in the database.
pub(crate) async fn load_world(ctx: &DaemonContext) -> SkinWorld {
    let mut world = SkinWorld::default();
    if let Some(session) = ctx.session.read().as_ref() {
        for f in session.friends.values() {
            if f.friendship_state == FriendshipState::Accepted {
                world.accepted_friends += 1;
                let server = f.game_info.as_ref().and_then(|g| g.server_address.as_ref());
                world.friend_servers += usize::from(server.is_some());
            }
        }
        world.communities = session.communities.len();
    }
    world.own_game = ctx.own_presence.view().game.map(|g| g.game_id);
    let (Ok(db), Ok(owner)) = (ctx.require_db(), ctx.require_owner()) else {
        return world;
    };
    let now = ctx.clock.now_ms_i64();
    let read = db
        .call(move |conn| {
            use rekindle_db::repo::{favourite_servers, group_dm_invites, group_dms};
            let favourites = favourite_servers::list(conn, &owner)?;
            let groups = group_dms::list(conn, &owner)?;
            let invites = group_dm_invites::list(conn, &owner, now)?;
            let joined = groups.into_iter().map(|g| {
                let others = g.roster.iter().filter(|m| m.identity != owner).count();
                (g.group_id, Group::Joined { others })
            });
            let pending = invites.into_iter().map(|i| (i.group_id, Group::Pending));
            let favourites: BTreeSet<(u32, String)> = favourites
                .into_iter()
                .map(|f| (f.game_id, f.address))
                .collect();
            let groups: BTreeMap<String, Group> = pending.chain(joined).collect();
            Ok((favourites, groups))
        })
        .await;
    match read {
        Ok((favourites, groups)) => {
            world.favourites = favourites;
            world.groups = groups;
        }
        Err(e) => tracing::warn!(error = %e, "skin signals: database state unread"),
    }
    world
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

    /// A download's state changed (`AttachmentDownload`).
    pub(crate) fn set_transfer(&self, attachment_id: &str, transfer: Transfer) {
        self.inputs.lock().set_transfer(attachment_id, transfer);
        self.poke();
    }

    /// The shell's window focus (`Host::skin_window_focused`).
    pub(crate) fn set_focus(&self, window: &str, focused: bool) {
        self.inputs.lock().set_focus(window, focused);
        self.poke();
    }
}

/// Each window's set that differs from the one last published.
pub(crate) fn changed(ctx: &DaemonContext, world: &SkinWorld) -> Vec<(String, SkinSignalSet)> {
    let sets = {
        let session = ctx.session.read();
        let inputs = ctx.skin.signals.inputs.lock();
        window_sets(&inputs, &ctx.skin.selection, world, |k| {
            session.as_ref().and_then(|s| s.friends.get(k).cloned())
        })
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
pub(crate) fn publish_changes(ctx: &DaemonContext, world: &SkinWorld) {
    for (window, set) in changed(ctx, world) {
        ctx.journal.publish_state(
            &format!("skin-signals:{window}"),
            SubscriptionEvent::System(SystemEvent::SkinSignals { window, set }),
        );
    }
}

/// The worker: fold the Host's stream and the pokes until shutdown.
pub(crate) async fn run_worker(ctx: Arc<DaemonContext>) {
    let mut stream = ctx.journal.subscribe(None);
    publish_changes(&ctx, &load_world(&ctx).await);
    loop {
        tokio::select! {
            () = ctx.shutdown.requested() => return,
            frame = stream.next() => match frame {
                Some(StreamFrame::Event(s)) => {
                    if !ctx.skin.signals.inputs.lock().fold(&s.event) {
                        continue;
                    }
                }
                Some(_) => {}
                None => return,
            },
            () = ctx.skin.signals.poke.notified() => {}
        }
        publish_changes(&ctx, &load_world(&ctx).await);
    }
}

impl crate::host::Host {
    /// The shell's window focus for the skin window labelled `window`
    /// (the Tauri window-focus event): it drives the window's
    /// `OnActivate`/`OnDeactivate`.
    pub fn skin_window_focused(&self, window: &str, focused: bool) {
        self.ctx.skin.signals.set_focus(window, focused);
    }

    /// The shell opened a skin window; `selection` is what it shows from
    /// the start (a chat window's own peer, lead decision 48).
    pub fn skin_window_opened(
        &self,
        window: &str,
        selection: Option<rekindle_types::dto::skin_run::SkinSelection>,
    ) {
        self.ctx.skin.signals.inputs.lock().open(window);
        self.ctx.skin.selection.set(window, selection);
        self.ctx.skin.signals.poke();
    }

    /// A skin window closed: its selection and signals go with it.
    pub fn skin_window_closed(&self, window: &str) {
        self.ctx.skin.signals.inputs.lock().close(window);
        self.ctx.skin.selection.set(window, None);
        self.ctx
            .journal
            .clear_state(&format!("skin-signals:{window}"));
        self.ctx.skin.signals.poke();
    }

    /// Our microphone mute in the call, from the desktop call engine
    /// (until E4.1 moves calls into the Host): `OnAppCanMuteSelf` /
    /// `OnAppCanUnmuteSelf`.
    pub fn skin_self_muted(&self, muted: bool) {
        self.ctx.skin.signals.inputs.lock().set_self_muted(muted);
        self.ctx.skin.signals.poke();
    }
}

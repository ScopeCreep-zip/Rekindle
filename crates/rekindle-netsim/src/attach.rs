//! Attachment on the simulated network (plan E2.B S9). Veilid reports two
//! separate things. The attachment level (`AttachmentState`,
//! `veilid_api/types/veilid_state.rs:6-24`) steps Attaching → AttachedWeak
//! … AttachedFull as the routing table fills, and Detaching → Detached on
//! the way down; `is_attached` holds from AttachedWeak to AttachedFull
//! (`veilid_state.rs:31-41`). Public-internet readiness
//! (`RoutingDomainReadyEvent`, `routing_table/health.rs:255-287`) gates the
//! DHT (`storage_manager/mod.rs:887-898`) while the node stays attached;
//! that split is the live log's "PublicInternet ready / unavailable" flap.
//!
//! Production turns each `VeilidUpdate::Attachment` into a
//! `PortEvent::Attachment` whose `public_internet_ready` drives the ready
//! watch (`rekindle-transport` `SharedState::set_attachment`); here the
//! ready watch follows the same event when the pump delivers it
//! ([`Network::deliver`]).
//!
//! Veilid 0.5.8 (`/Users/kali/veilid-wt-058`):
//! - **a stall re-sends:** a tick that comes more than 2 s late records a
//!   local stall (`attachment_manager/mod.rs:557-566`); at
//!   [`LOCAL_STALL_MIN`] or more the manager asks for a resend
//!   (`:578-581`, `network_manager/online_detector.rs:710-713`), and the
//!   next update is sent even though nothing changed (`:1036-1043`,
//!   `:1117-1121`) ([`crate::Fault::Stall`]). An app in the background
//!   hears it on its return instead; the netsim's nodes are foreground.
//! - **readiness returns after a round trip:** leaving Offline needs the
//!   answer to a question sent after going offline
//!   (`online_detector.rs:1337-1352`), so a public internet that turns
//!   ready again is ready for the node one round trip later
//!   ([`READY_ROUND_TRIP`]): its DHT keeps answering "try again" and its
//!   update arrives then.

use std::time::Duration;

use rekindle_records::port::PortEvent;

use crate::network::Network;
use crate::{toggle, Netsim};

/// The gap between two attachment updates of one node: each is its own
/// delivery, in order (Veilid posts them one after another).
pub const ATTACH_STEP: Duration = Duration::from_millis(250);

/// The shortest stall after which Veilid re-sends the attachment state: half
/// the RPC timeout (`network_manager/online_detector.rs:63`).
pub const LOCAL_STALL_MIN: Duration = Duration::from_millis(2_500);

/// The round trip that proves a returned public internet: two
/// [`crate::latency::MESSAGE`] legs at its mean (a fixed span, so it draws
/// nothing from the seed).
pub const READY_ROUND_TRIP: Duration = Duration::from_millis(1_000);

/// The levels an attach steps through and whether the public internet is
/// ready at each: not while Attaching, and not yet at the weakest level.
const UP: [(&str, bool); 6] = [
    ("attaching", false),
    ("attached_weak", false),
    ("attached_fair", true),
    ("attached_good", true),
    ("attached_strong", true),
    ("attached_full", true),
];

/// The level a reattach settles at (fewer peers than a fresh start).
const REATTACH_TOP: usize = 3;

impl Network {
    /// Queue `node`'s attachment update after its last one.
    pub(crate) fn attachment(&self, node: usize, state: &'static str, ready: bool) {
        self.attachment_after(node, state, ready, Duration::ZERO);
    }

    /// Queue `node`'s attachment update after its last one, and at least
    /// `after` from now.
    pub(crate) fn attachment_after(
        &self,
        node: usize,
        state: &'static str,
        ready: bool,
        after: Duration,
    ) {
        let delay = {
            let mut inner = self.inner.lock();
            let Some(slot) = inner.nodes.get_mut(node) else {
                return;
            };
            let now = tokio::time::Instant::now();
            let at = slot.attach_next.max(now + after);
            slot.attach_next = at + ATTACH_STEP;
            slot.state = state;
            slot.public_ready = ready;
            at - now
        };
        let attached = state.starts_with("attached_");
        let (peers, size, latency) = if attached { (8, 64, 80_000) } else { (0, 0, 0) };
        self.send_after(
            node,
            node,
            "Attachment",
            PortEvent::Attachment {
                state: state.into(),
                is_attached: attached,
                public_internet_ready: ready,
                reliable_peers: peers,
                live_peers: peers,
                network_size: size,
                median_latency_us: latency,
            },
            false,
            delay,
        );
    }

    fn step_up(&self, node: usize, top: usize) {
        for (state, ready) in &UP[..=top] {
            self.attachment(node, state, *ready);
        }
    }

    fn step_down(&self, node: usize) {
        self.attachment(node, "detaching", false);
        self.attachment(node, "detached", false);
    }
}

impl Netsim {
    /// Attach `node` (Attaching up to AttachedFull, ready from
    /// AttachedFair) or take it down (Detaching, then Detached). Its ready
    /// watch follows each update as it is delivered.
    pub fn set_ready(&self, node: usize, ready: bool) {
        if ready {
            self.net.step_up(node, UP.len() - 1);
        } else {
            self.net.step_down(node);
        }
    }

    /// [`crate::Fault::Detach`].
    pub(crate) fn detach(&self, node: usize, detached: bool) {
        {
            let mut inner = self.net.inner.lock();
            toggle(&mut inner.faults.dht_offline, node, detached);
            toggle(&mut inner.faults.detached, node, detached);
            inner.faults.not_ready.remove(&node);
            inner.faults.ready_pending.remove(&node);
        }
        if detached {
            self.net.step_down(node);
        } else {
            self.net.step_up(node, REATTACH_TOP);
        }
    }

    /// [`crate::Fault::InternetReady`]: the node stays attached at its
    /// level; a detached node has no routing domain to change. Unready
    /// holds at once; ready holds from its update, [`READY_ROUND_TRIP`]
    /// later ([`Network::deliver`] ends `not_ready` as it delivers it).
    pub(crate) fn internet_ready(&self, node: usize, ready: bool) {
        let state = {
            let mut inner = self.net.inner.lock();
            if inner.faults.detached.contains(&node) {
                return;
            }
            if ready {
                if !inner.faults.not_ready.contains(&node) {
                    return;
                }
                inner.faults.ready_pending.insert(node);
            } else {
                inner.faults.not_ready.insert(node);
                inner.faults.ready_pending.remove(&node);
            }
            match inner.nodes.get(node) {
                Some(slot) => slot.state,
                None => return,
            }
        };
        let after = if ready {
            READY_ROUND_TRIP
        } else {
            Duration::ZERO
        };
        self.net.attachment_after(node, state, ready, after);
    }

    /// [`crate::Fault::Stall`]: from [`LOCAL_STALL_MIN`] on, the node hears
    /// its attachment state again, unchanged; a detached node has no
    /// attachment manager tick to stall.
    pub(crate) fn stall(&self, node: usize, lag: Duration) {
        if lag < LOCAL_STALL_MIN {
            return;
        }
        let (state, ready) = {
            let inner = self.net.inner.lock();
            if inner.faults.detached.contains(&node) {
                return;
            }
            match inner.nodes.get(node) {
                Some(slot) => (slot.state, slot.public_ready),
                None => return,
            }
        };
        self.net.attachment(node, state, ready);
    }
}

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

use std::time::Duration;

use rekindle_records::port::PortEvent;

use crate::network::Network;
use crate::{toggle, Netsim};

/// The gap between two attachment updates of one node: each is its own
/// delivery, in order (Veilid posts them one after another).
pub const ATTACH_STEP: Duration = Duration::from_millis(250);

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
        let delay = {
            let mut inner = self.inner.lock();
            let Some(slot) = inner.nodes.get_mut(node) else {
                return;
            };
            let now = tokio::time::Instant::now();
            let at = slot.attach_next.max(now);
            slot.attach_next = at + ATTACH_STEP;
            slot.state = state;
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
        }
        if detached {
            self.net.step_down(node);
        } else {
            self.net.step_up(node, REATTACH_TOP);
        }
    }

    /// [`crate::Fault::InternetReady`]: the node stays attached at its
    /// level; a detached node has no routing domain to change.
    pub(crate) fn internet_ready(&self, node: usize, ready: bool) {
        let state = {
            let mut inner = self.net.inner.lock();
            if inner.faults.detached.contains(&node) {
                return;
            }
            toggle(&mut inner.faults.not_ready, node, !ready);
            match inner.nodes.get(node) {
                Some(slot) => slot.state,
                None => return,
            }
        };
        self.net.attachment(node, state, ready);
    }
}

//! A node restart (plan E3.2 S7, NR1): the node's process ends and starts
//! again over the same data root, as Veilid 0.5.8 terminates and starts its
//! storage manager (`~/veilid-wt-058/veilid-core/src/storage_manager`).
//!
//! **Kept**, on disk in the node's table store:
//! - the local and remote record stores, flushed at termination
//!   (`mod.rs:655-672`) and opened again at startup (`mod.rs:513-525`): the
//!   node's local copies and their seqs (`NodeSlot::local`,
//!   `NodeSlot::known`, their nonces);
//! - the offline subkey writes, the rehydration requests and the outbound
//!   watches, saved at termination (`mod.rs:674-677`, `:719-763`) and loaded
//!   at startup (`mod.rs:550`, `:807-885`): `NodeSlot::pushes`,
//!   `NodeSlot::rehydrations`, `NodeSlot::watches` and
//!   `NodeSlot::outbound`;
//! - the node's allocated private routes, which the route spec store loads
//!   again (`routing_table/route_spec_store/mod.rs:157-187`).
//!
//! **Cleared**, in memory only:
//! - the opened records and the records created this session
//!   (`locally_created_records`), reset at termination (`mod.rs:681-695`):
//!   a record the node made before the restart is no longer "created this
//!   session", so an open that asks for it rehydrates it
//!   (`open_record.rs:62-74`);
//! - the rehydration checks, an LRU the storage manager builds when it is
//!   made (`mod.rs:265`, `:402`);
//! - the sets in flight, whose outbound transactions end at termination
//!   (`mod.rs:649-650`);
//! - the remote route cache, which the route spec store builds anew at load
//!   (`route_spec_store/mod.rs:160`).
//!
//! The process hears nothing while it is down: events in flight to it are
//! dropped, and to the other nodes it is detached as in [`crate::attach`]
//! (calls to it time out, no change reaches it). It starts with new event
//! and ready channels and attaches from Attaching up, as at
//! [`Netsim::start`]; its clock, idle port and generator carry on.

use std::sync::Arc;

use tokio::sync::{mpsc, watch};

use crate::network::EVENT_QUEUE;
use crate::{Netsim, Node};

impl Netsim {
    /// `node`'s process ends (its Host has shut down): its Veilid
    /// terminates as the module notes say, and it is detached until
    /// [`Self::start_node`].
    pub fn terminate_node(&self, node: usize) {
        let mut inner = self.net.inner.lock();
        inner.faults.detached.insert(node);
        inner.faults.dht_offline.insert(node);
        inner.faults.not_ready.remove(&node);
        inner.faults.ready_pending.remove(&node);
        inner.drop_in_flight_to(node);
        let Some(slot) = inner.nodes.get_mut(node) else {
            return;
        };
        // The ended process's channels go with it.
        slot.events = mpsc::channel(EVENT_QUEUE).0;
        slot.ready = watch::channel(false).0;
        slot.state = "detached";
        slot.public_ready = false;
        slot.opened.clear();
        slot.created.clear();
        slot.rehydration_checked.clear();
        slot.setting.clear();
        slot.imported.clear();
        slot.pools.clear();
    }

    /// `node`'s process starts again over the data [`Self::terminate_node`]
    /// kept: new ports and event stream for a Host, attaching from
    /// Attaching up.
    #[must_use]
    pub fn start_node(&self, node: usize) -> Node {
        let (events_tx, events) = mpsc::channel(EVENT_QUEUE);
        let (ready, ready_rx) = watch::channel(false);
        {
            let mut inner = self.net.inner.lock();
            inner.faults.detached.remove(&node);
            inner.faults.dht_offline.remove(&node);
            // Its first attach proves the network by a round trip
            // (`online_detector.rs:1346-1348`, [`crate::loopback`]).
            inner.faults.detector_offline.remove(&node);
            if let Some(slot) = inner.nodes.get_mut(node) {
                slot.events = events_tx;
                slot.ready = ready;
            }
        }
        let rng = Arc::clone(&self.rngs.lock()[node]);
        let clock = Arc::clone(&self.clocks.lock()[node]);
        let idle = Arc::clone(&self.idles.lock()[node]);
        let ports = self.ports(node, &rng, ready_rx);
        self.set_ready(node, true);
        Node {
            index: node,
            ports,
            events,
            clock,
            rng,
            idle,
        }
    }
}

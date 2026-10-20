//! Records on the simulated DHT (plan C9.3): `FakeRecords` starts a
//! `FakePool` per unlocked session, as the Veilid store does.
//!
//! The model follows Veilid where the app depends on it:
//! - a created record is local to its creator until its first write lands
//!   (publish on first write);
//! - a write is signed by a writer the schema admits (the owner for owner
//!   subkeys, the member for a member's), checked through
//!   `parse_keypair_text`, and the writer is sticky per record per node;
//! - a write lands at the node's local seq + 1, and a network value at that
//!   seq or newer supersedes it (and becomes the local copy);
//! - each node keeps local copies across its sessions; reads refresh them
//!   from the network unless the stale-read fault holds;
//! - watches are per node, the union of the session's borrowers, and end
//!   with the session.

use std::collections::BTreeSet;
use std::sync::Arc;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use parking_lot::Mutex;
use rekindle_records::lease::{max_subkey_bytes, LeaseId, LeaseTable, SchemaShape, WatchPlan};
use rekindle_records::port::{
    PortError, RecordPort, RecordSchema, RecordStore, RecordValue, SetOutcome, Settled,
};
use rekindle_utils::RandomSource;

use crate::clock::SimRandom;
use crate::network::{NetRecord, Network};

mod port;

/// The record store of one simulated node.
pub struct FakeRecords {
    node: usize,
    net: Arc<Network>,
    rng: Arc<SimRandom>,
}

impl FakeRecords {
    pub(crate) fn new(node: usize, net: Arc<Network>, rng: Arc<SimRandom>) -> Self {
        Self { node, net, rng }
    }
}

impl RecordStore for FakeRecords {
    fn start_session(&self) -> Result<Arc<dyn RecordPort>, PortError> {
        let pool = Arc::new(FakePool {
            node: self.node,
            net: Arc::clone(&self.net),
            rng: Arc::clone(&self.rng),
            state: Mutex::new(PoolState::default()),
            settled: tokio::sync::broadcast::channel(64).0,
        });
        if let Some(slot) = self.net.inner.lock().nodes.get_mut(self.node) {
            slot.pools.push(Arc::downgrade(&pool));
        }
        Ok(pool)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Phase {
    #[default]
    Open,
    /// Lock began: new calls are refused.
    Draining,
    /// The lock's own writes are admitted on the drained pool.
    Teardown,
    Ended,
}

#[derive(Debug, Clone)]
struct Held {
    key: String,
    subkey: u32,
    data: Vec<u8>,
    writer: Option<String>,
}

#[derive(Default)]
struct PoolState {
    leases: LeaseTable<String>,
    held: Vec<Held>,
    phase: Phase,
}

/// One session's records on the simulated DHT.
pub struct FakePool {
    node: usize,
    net: Arc<Network>,
    rng: Arc<SimRandom>,
    state: Mutex<PoolState>,
    settled: tokio::sync::broadcast::Sender<Settled>,
}

/// A record key in Veilid's text form, from the node's generator.
pub(crate) fn new_key_text(rng: &dyn RandomSource) -> String {
    let mut bytes = [0u8; 32];
    rng.fill(&mut bytes);
    format!("VLD0:{}", URL_SAFE_NO_PAD.encode(bytes))
}

/// A public key in Veilid's text form (`RecordValue::writer`).
pub(crate) fn public_text(public: &[u8; 32]) -> String {
    format!("VLD0:{}", URL_SAFE_NO_PAD.encode(public))
}

/// The public half of a writer's keypair text, after checking that its
/// secret is that public key's.
pub(crate) fn writer_public(text: &str) -> Result<[u8; 32], PortError> {
    let (public, secret) = rekindle_crypto::keypair_text::parse_keypair_text(text)
        .map_err(|e| PortError::KeyText(format!("keypair: {e}")))?;
    let derived = rekindle_secrets::ed25519_dalek::SigningKey::from_bytes(&secret)
        .verifying_key()
        .to_bytes();
    if derived != public {
        return Err(PortError::KeyText("keypair halves do not match".into()));
    }
    Ok(public)
}

/// Whether `writer` may write `subkey` under `record`'s schema.
pub(crate) fn admits(record: &NetRecord, subkey: u32, writer: &[u8; 32]) -> bool {
    match &record.schema {
        RecordSchema::Dflt { owner_subkeys } => {
            subkey < u32::from(*owner_subkeys) && *writer == record.owner
        }
        RecordSchema::Smpl {
            owner_subkeys,
            members,
        } => {
            if subkey < u32::from(*owner_subkeys) {
                return *writer == record.owner;
            }
            let mut first = u32::from(*owner_subkeys);
            for member in members {
                let end = first + u32::from(member.subkeys);
                if subkey < end {
                    return member.key == *writer;
                }
                first = end;
            }
            false
        }
    }
}

fn subkey_count(schema: &RecordSchema) -> u32 {
    u32::try_from(schema.subkey_count()).unwrap_or(u32::MAX)
}

impl FakePool {
    fn check_open(&self, write: bool) -> Result<(), PortError> {
        match self.state.lock().phase {
            Phase::Open => Ok(()),
            Phase::Teardown if write => Ok(()),
            Phase::Draining | Phase::Teardown | Phase::Ended => Err(PortError::Closed),
        }
    }

    fn key(&self, lease: LeaseId) -> Result<String, PortError> {
        self.state
            .lock()
            .leases
            .key(lease)
            .map(ToString::to_string)
            .ok_or(PortError::LeaseNotHeld(lease.0))
    }

    /// The DHT reach faults: `Err` when this node cannot reach the network
    /// for this call.
    fn reach(&self, key: &str) -> Result<(), PortError> {
        let mut inner = self.net.inner.lock();
        if inner.faults.dht_offline.contains(&self.node) {
            return Err(PortError::Unreachable(key.to_string()));
        }
        if let Some(left) = inner.faults.try_again.get_mut(&self.node) {
            if *left > 0 {
                *left -= 1;
                return Err(PortError::Unreachable(key.to_string()));
            }
        }
        Ok(())
    }

    fn apply_watch(&self, key: &str, plan: WatchPlan) {
        let mut inner = self.net.inner.lock();
        let Some(slot) = inner.nodes.get_mut(self.node) else {
            return;
        };
        match plan {
            WatchPlan::Unchanged => {}
            WatchPlan::Watch(subkeys) => {
                slot.watches.insert(key.to_string(), subkeys);
            }
            WatchPlan::Cancel => {
                slot.watches.remove(key);
            }
        }
    }

    fn local(&self, key: &str, subkey: u32) -> Option<RecordValue> {
        self.net
            .inner
            .lock()
            .nodes
            .get(self.node)?
            .local
            .get(&(key.to_string(), subkey))
            .cloned()
    }

    /// Refresh the local copy of `subkey` from the network when the network
    /// is newer; the local copy after.
    fn refresh(&self, key: &str, subkey: u32) -> Option<RecordValue> {
        let mut inner = self.net.inner.lock();
        let network = inner
            .records
            .get(key)
            .and_then(|r| r.values.get(subkey as usize).cloned().flatten());
        let slot = inner.nodes.get_mut(self.node)?;
        let local = slot.local.get(&(key.to_string(), subkey)).cloned();
        match (network, local) {
            (Some(n), Some(l)) if n.seq > l.seq => {
                slot.local.insert((key.to_string(), subkey), n.clone());
                Some(n)
            }
            (Some(n), None) => {
                slot.local.insert((key.to_string(), subkey), n.clone());
                Some(n)
            }
            (_, local) => local,
        }
    }

    /// One write, the honest-write outcome Veilid gives (see the module
    /// documentation).
    fn write(
        &self,
        key: &str,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<&str>,
    ) -> Result<SetOutcome, PortError> {
        let sticky = self.state.lock().leases.writer(key).cloned();
        let writer = writer
            .map(ToString::to_string)
            .or(sticky)
            .ok_or_else(|| PortError::NotWritable(key.to_string()))?;
        let public = writer_public(&writer)?;
        {
            let inner = self.net.inner.lock();
            let record = inner
                .records
                .get(key)
                .ok_or_else(|| PortError::Unreachable(key.to_string()))?;
            let count = subkey_count(&record.schema);
            if subkey >= count {
                return Err(PortError::Dht(format!("subkey {subkey} out of range")));
            }
            let cap = max_subkey_bytes(SchemaShape {
                subkey_count: count,
            });
            if data.len() > cap {
                return Err(PortError::SubkeyTooLarge {
                    subkey,
                    len: data.len(),
                    cap,
                });
            }
            if !admits(record, subkey, &public) {
                return Err(PortError::NotWritable(key.to_string()));
            }
        }
        if self.reach(key).is_err() {
            return Ok(SetOutcome::Offline);
        }
        let landed = {
            let mut inner = self.net.inner.lock();
            let network = inner
                .records
                .get(key)
                .and_then(|r| r.values.get(subkey as usize).cloned().flatten());
            let slot = &mut inner.nodes[self.node];
            let local = slot.local.get(&(key.to_string(), subkey)).cloned();
            if let Some(l) = local.as_ref().filter(|l| l.data == data) {
                // The local copy already: re-push it if the network is behind.
                let behind = network.as_ref().is_none_or(|n| n.seq < l.seq);
                let pushed = l.clone();
                if behind {
                    if let Some(r) = inner.records.get_mut(key) {
                        r.values[subkey as usize] = Some(pushed.clone());
                        r.published = true;
                    }
                    drop(inner);
                    self.net.notify_watchers(self.node, key, subkey, &pushed);
                }
                return Ok(SetOutcome::Unchanged);
            }
            let seq = local.map_or(0, |l| l.seq.saturating_add(1));
            if let Some(newer) = network.filter(|n| n.seq >= seq) {
                slot.local.insert((key.to_string(), subkey), newer.clone());
                return Ok(SetOutcome::Superseded(newer));
            }
            let value = RecordValue {
                seq,
                data,
                writer: public_text(&public),
            };
            slot.local.insert((key.to_string(), subkey), value.clone());
            if let Some(r) = inner.records.get_mut(key) {
                r.values[subkey as usize] = Some(value.clone());
                r.published = true;
            }
            value
        };
        self.net.notify_watchers(self.node, key, subkey, &landed);
        Ok(SetOutcome::Landed)
    }

    /// Forget the held writes, as a killed process loses them: they lived
    /// only in memory, so nothing re-pushes them.
    pub(crate) fn forget_held(&self) {
        self.state.lock().held.clear();
    }

    /// Re-push the held writes (the network came back): those that land or
    /// lose settle.
    pub(crate) fn retry_held(&self) {
        let held = std::mem::take(&mut self.state.lock().held);
        let mut keep = Vec::new();
        for write in held {
            let outcome = self.write(
                &write.key,
                write.subkey,
                write.data.clone(),
                write.writer.as_deref(),
            );
            let landed = match outcome {
                Ok(SetOutcome::Landed | SetOutcome::Unchanged) => Some(true),
                Ok(SetOutcome::Superseded(_)) => Some(false),
                Ok(SetOutcome::Offline | SetOutcome::BelowConsensus) | Err(_) => None,
            };
            match landed {
                Some(landed) => {
                    let _ = self.settled.send(Settled {
                        record_key: write.key.clone(),
                        subkey: write.subkey,
                        landed,
                    });
                }
                None => keep.push(write),
            }
        }
        self.state.lock().held.extend(keep);
    }
}

/// Every watched subkey of `key` on `node`, for a fault that kills it.
pub(crate) fn watched_subkeys(net: &Network, node: usize, key: &str) -> BTreeSet<u32> {
    net.inner
        .lock()
        .nodes
        .get(node)
        .and_then(|slot| slot.watches.get(key).cloned())
        .unwrap_or_default()
}

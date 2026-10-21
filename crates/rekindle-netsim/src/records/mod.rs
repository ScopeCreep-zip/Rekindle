//! Records on the simulated DHT (plan C9.3, E2.B slice S3): `FakeRecords`
//! starts the production `RecordPool` per unlocked session, over a
//! [`dht::SimDht`] backend, as the Veilid store does over `VeilidDht`.
//!
//! The model follows Veilid where the app depends on it ([`dht`] cites the
//! storage manager call by call):
//! - a created record is local to its creator until its first write lands
//!   (publish on first write);
//! - a write is signed by a writer the schema admits (the owner for owner
//!   subkeys, the member for a member's), checked through
//!   `parse_keypair_text`, and the open's writer is sticky per record per
//!   node;
//! - a write lands at the node's local seq + 1, and a network value at that
//!   seq or newer supersedes it (and becomes the local copy);
//! - each node keeps local copies across its sessions; reads refresh them
//!   from the network unless the stale-read fault holds; a watched value
//!   that arrives is stored locally;
//! - watches are per node, the union of the session's borrowers, and end
//!   with the record's close;
//! - every call that reaches the network costs Veilid's latency
//!   ([`crate::latency`]) before its effect lands, under the per-subkey
//!   locks and the node's DHT cap.

use std::collections::BTreeSet;
use std::sync::Arc;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rekindle_record_pool::{as_port, RecordCloser, RecordPool};
use rekindle_records::port::{PortError, RecordPort, RecordSchema, RecordStore};
use rekindle_utils::RandomSource;

use crate::clock::SimRandom;
use crate::network::{NetRecord, Network};

mod dht;

/// The record store of one simulated node.
pub struct FakeRecords {
    node: usize,
    net: Arc<Network>,
    rng: Arc<SimRandom>,
    /// The node's closer of ended sessions' records, as the transport
    /// node's.
    closer: Arc<RecordCloser>,
}

impl FakeRecords {
    pub(crate) fn new(node: usize, net: Arc<Network>, rng: Arc<SimRandom>) -> Self {
        Self {
            node,
            net,
            rng,
            closer: RecordCloser::new(),
        }
    }
}

impl RecordStore for FakeRecords {
    fn start_session(&self) -> Result<Arc<dyn RecordPort>, PortError> {
        let (backend, ready) = {
            let inner = self.net.inner.lock();
            let slot = inner
                .nodes
                .get(self.node)
                .ok_or_else(|| PortError::Node("no such node".into()))?;
            (
                dht::SimDht::new(self.node, Arc::clone(&self.net), Arc::clone(&self.rng)),
                slot.ready.subscribe(),
            )
        };
        // The pool's calls run on its own scope, as `VeilidRecords` does.
        let scope = rekindle_lifecycle::SessionScope::new(
            "record pool",
            Arc::new(|task| tracing::error!(task, "record pool task panicked")),
        );
        let pool = RecordPool::new(Arc::new(backend), scope, ready, Arc::clone(&self.closer));
        if let Some(slot) = self.net.inner.lock().nodes.get_mut(self.node) {
            slot.pools.push(Arc::downgrade(&pool));
        }
        Ok(as_port(pool))
    }
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

/// Every watched subkey of `key` on `node`, for a fault that kills it.
pub(crate) fn watched_subkeys(net: &Network, node: usize, key: &str) -> BTreeSet<u32> {
    net.inner
        .lock()
        .nodes
        .get(node)
        .and_then(|slot| slot.watches.get(key).cloned())
        .unwrap_or_default()
}

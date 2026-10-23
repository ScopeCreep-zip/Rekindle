//! Records on the simulated DHT (plan C9.3, E2.B slice S3): `FakeRecords`
//! starts the production `RecordPool` per unlocked session, over a
//! [`dht::SimDht`] backend, as the Veilid store does over `VeilidDht`.
//!
//! The model follows Veilid where the app depends on it ([`dht`] cites the
//! storage manager call by call):
//! - a created record is local to its creator until a write reaches one of
//!   its replicas (publish on first write);
//! - each record lives on seeded replicas; a partition splits which of
//!   them a node reaches, a replica keeps the first value at a seq, a set
//!   needs consensus and a get takes the newest ([`replicas`] cites Veilid);
//! - a write is signed by a writer the schema admits (the owner for owner
//!   subkeys, the member for a member's), checked through
//!   `parse_keypair_text`, and the open's writer is sticky per record per
//!   node;
//! - a write goes out at the node's local seq + 1 (the same seq for the
//!   same data and writer); below consensus the local copy stays, and a
//!   newer value a replica holds supersedes it (and becomes the local copy);
//! - each node keeps local copies across its sessions; reads refresh them
//!   from the replicas it reaches; a watched value that arrives is stored
//!   locally;
//! - watches are per node, the union of the session's borrowers, held by
//!   the replicas the node reaches within Veilid's limits, and end with the
//!   death shape on cancel, close or expiry ([`watches`] cites Veilid);
//! - a node inspects its watches when its DHT comes back online, after a
//!   tick lag, and when a watch is established or newly held, reporting
//!   what the network holds newer than its local copy ([`catchup`] cites
//!   Veilid);
//! - an open of a locally held record queues Veilid's rehydration, which
//!   pushes the local subkeys the replicas lack ([`rehydrate`] cites
//!   Veilid); each DHT store keeps at most Veilid's remote record count and
//!   evicts the least recently used record ([`stores`]);
//! - every call that reaches the network costs Veilid's latency
//!   ([`crate::latency`]) under the per-subkey locks and the node's DHT
//!   cap; a set lands on each replica as its RPC arrives ([`fanout`]);
//! - a value is within Veilid's size rule at every hop ([`size`]).

use std::sync::Arc;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rekindle_record_pool::{as_port, RecordCloser, RecordPool};
use rekindle_records::port::{PortError, RecordPort, RecordSchema, RecordStore, RecordValue};
use rekindle_utils::RandomSource;

use crate::clock::SimRandom;
use crate::network::{NetRecord, Network};

mod catchup;
#[cfg(test)]
mod catchup_tests;
mod dht;
mod fanout;
#[cfg(test)]
mod fanout_tests;
#[cfg(test)]
mod key_tests;
mod outside;
#[cfg(test)]
mod outside_tests;
mod rehydrate;
#[cfg(test)]
mod rehydrate_tests;
mod replicas;
#[cfg(test)]
mod report_tests;
pub(crate) mod size;
mod stores;
#[cfg(test)]
mod watch_tests;
pub(crate) mod watches;

pub(crate) use replicas::{reachable, reaches, Replica};

/// A get's answer over the `reach` replicas with no local copy.
pub(crate) fn get_from(record: &NetRecord, reach: &[usize], subkey: u32) -> Option<RecordValue> {
    replicas::get(
        &record.schema,
        &record.replicas,
        reach,
        subkey as usize,
        None,
    )
}

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

/// Veilid's crypto kind `VLD0`, the first bytes a record key hashes.
const CRYPTO_KIND: [u8; 4] = *b"VLD0";

/// A schema's compiled bytes, as Veilid's `DHTSchema::compile`
/// (`schema/dflt.rs:48-55`, `schema/smpl.rs:106-122`): the kind's four
/// bytes, `o_cnt` little-endian, then each SMPL member's id and `m_cnt`
/// little-endian.
pub(crate) fn schema_bytes(schema: &RecordSchema) -> Vec<u8> {
    match schema {
        RecordSchema::Dflt { owner_subkeys } => {
            let mut out = b"DFLT".to_vec();
            out.extend_from_slice(&owner_subkeys.to_le_bytes());
            out
        }
        RecordSchema::Smpl {
            owner_subkeys,
            members,
        } => {
            let mut out = b"SMPL".to_vec();
            out.extend_from_slice(&owner_subkeys.to_le_bytes());
            for member in members {
                out.extend_from_slice(&member.key);
                out.extend_from_slice(&member.subkeys.to_le_bytes());
            }
            out
        }
    }
}

/// A record key in Veilid's text form, derived as Veilid derives it
/// (`storage_manager/record_key.rs:67-94`): the opaque key is BLAKE3 over
/// the crypto kind, the owner's public key and the compiled schema, and the
/// key text is `VLD0:<opaque>:<encryption key>`, both base64url without
/// padding (`crypto_typed.rs:76-81`, `record_key.rs:61-67`).
pub(crate) fn record_key_text(
    owner: &[u8; 32],
    schema: &RecordSchema,
    encryption_key: &[u8; 32],
) -> String {
    let mut hash_data = CRYPTO_KIND.to_vec();
    hash_data.extend_from_slice(owner);
    hash_data.extend_from_slice(&schema_bytes(schema));
    let opaque = blake3::hash(&hash_data);
    format!(
        "VLD0:{}:{}",
        URL_SAFE_NO_PAD.encode(opaque.as_bytes()),
        URL_SAFE_NO_PAD.encode(encryption_key)
    )
}

/// A record key text in canonical form, or `None` when it is not
/// `VLD0:<opaque>` or `VLD0:<opaque>:<encryption key>` with each part 32
/// bytes (`record_key.rs:100-122`, `check_record_key`).
pub(crate) fn canonical_key_text(key: &str) -> Option<String> {
    let part = |text: &str| {
        URL_SAFE_NO_PAD
            .decode(text)
            .ok()
            .filter(|b| b.len() == 32)
            .map(|b| URL_SAFE_NO_PAD.encode(b))
    };
    let mut parts = key.strip_prefix("VLD0:")?.split(':');
    let opaque = part(parts.next()?)?;
    let canonical = match parts.next() {
        None => format!("VLD0:{opaque}"),
        Some(secret) => format!("VLD0:{opaque}:{}", part(secret)?),
    };
    parts.next().is_none().then_some(canonical)
}

/// A fresh record encryption key from the node's generator, as Veilid's
/// `random_shared_secret` in `create_record.rs:88-89`.
pub(crate) fn new_encryption_key(rng: &dyn RandomSource) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    rng.fill(&mut bytes);
    bytes
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

/// Veilid's schema checks (`schema/mod.rs:22-24`, `schema/dflt.rs:30-35`,
/// `schema/smpl.rs:31,49-90`): 1 to 1024 subkeys, DFLT `o_cnt > 0`; SMPL at
/// most 256 members and 256 writers (the owner when it has subkeys, and
/// each member with subkeys).
pub(crate) fn validate_schema(schema: &RecordSchema) -> Result<(), String> {
    const MAX_SUBKEYS: usize = 1024;
    const MAX_MEMBERS: usize = 256;
    const MAX_WRITERS: usize = 256;
    match schema {
        RecordSchema::Dflt { owner_subkeys: 0 } => return Err("o_cnt is 0".into()),
        RecordSchema::Dflt { .. } => {}
        RecordSchema::Smpl {
            owner_subkeys,
            members,
        } => {
            if members.len() > MAX_MEMBERS {
                return Err(format!("{} members", members.len()));
            }
            let writers: std::collections::HashSet<[u8; 32]> = members
                .iter()
                .filter(|m| m.subkeys > 0)
                .map(|m| m.key)
                .collect();
            let writers = writers.len() + usize::from(*owner_subkeys > 0);
            if writers > MAX_WRITERS {
                return Err(format!("{writers} writers"));
            }
        }
    }
    let count = schema.subkey_count();
    if count == 0 || count > MAX_SUBKEYS {
        return Err(format!("{count} subkeys"));
    }
    Ok(())
}

/// A set in flight on a node, until dropped.
pub(crate) struct Setting<'a> {
    net: &'a Network,
    node: usize,
    at: (String, u32),
}

impl<'a> Setting<'a> {
    pub(crate) fn new(net: &'a Network, node: usize, key: &str, subkey: u32) -> Self {
        let at = (key.to_string(), subkey);
        *net.inner.lock().nodes[node]
            .setting
            .entry(at.clone())
            .or_default() += 1;
        Self { net, node, at }
    }
}

impl Drop for Setting<'_> {
    fn drop(&mut self) {
        let mut inner = self.net.inner.lock();
        let setting = &mut inner.nodes[self.node].setting;
        if let Some(n) = setting.get_mut(&self.at) {
            *n -= 1;
            if *n == 0 {
                setting.remove(&self.at);
            }
        }
    }
}

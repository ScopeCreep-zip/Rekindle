//! `SimDht`: the netsim's [`DhtBackend`] (plan E2.B slice S3), one per
//! session pool, over the node's side of the simulated storage manager.
//!
//! The network side is the record's replicas (S4, [`super::replicas`]).
//! Each call follows the pinned veilid-core (`~/veilid/veilid-core/src/
//! storage_manager`) where the pool depends on it:
//!
//! - every call but create needs the record open on the node, else
//!   `InvalidArgument` "record not open" (`get_value.rs:67`,
//!   `set_value.rs:74-76`, `inspect_record.rs:112`, `watch_value.rs:136`);
//! - an open of a record in the node's local store is local
//!   (`open_record.rs:27-44`); any other open needs the network
//!   (`TryAgain` offline, `:48-49`) and finds nothing for a record nobody
//!   published (`KeyNotFound`, `:71-78`); a re-open replaces the writer;
//! - a get without `force_refresh` answers the local copy when there is one
//!   (`get_value.rs:79-88`), else asks the network, `TryAgain` offline
//!   (`:89-92`);
//! - a set signs with the given writer or the open's, else `Generic` "value
//!   is not writable" (`set_value.rs:91-93`); a writer the schema refuses is
//!   `Generic` "failed schema validation" (`set_value.rs:236-242`); the same
//!   data and writer keep the seq (`:212-218`), anything else is local seq
//!   + 1; offline with `allow_offline` false is `TryAgain` and stores
//!   nothing (`set_value.rs:109-115`,
//!   `local_record_store_interface.rs:61-62`); a fanout below consensus is
//!   `Ok(None)` with the local copy untouched while the replicas that took
//!   the value keep it (`set_value.rs:170-171,596-623`,
//!   `local_record_store_interface.rs:61-62`); at consensus the fanout's
//!   value becomes the local copy and is returned when it is not the one
//!   asked for (a newer value a replica held, `set_value.rs:620-645`), an
//!   identical re-push included;
//! - a get over the network takes the newest value the reached replicas
//!   hold and stores it locally when its seq moved
//!   (`get_value.rs:538-567`);
//! - an inspect in the local scope never leaves the node; any other is
//!   `TryAgain` offline (`inspect_record.rs:205-206`); UpdateSet reports
//!   each local seq + 1, and the Update scopes' network seqs start from the
//!   local result, the Sync ones' from nothing (`:208-228`), then take the
//!   newest seq of the replicas the node reaches; `offline_subkeys` is the
//!   node's own push queue plus its sets in flight (`:174-181`);
//! - a create checks the schema as Veilid ([`super::validate_schema`]);
//! - a watch's expiration is absolute: too soon on the node's own clock
//!   (its skew), clamped by each replica on true time;
//! - an open of a locally held record queues a rehydration
//!   ([`super::rehydrate`]); a call that reads or writes replicas uses the
//!   record on their stores ([`super::stores`]);
//! - a watch records the wanted state on the node, answers whether one is
//!   wanted and is held by the replicas the node reaches; a close is
//!   node-wide and ends the record's watch ([`super::watches`]).
//!
//! Every call that reaches the network costs Veilid's latency
//! ([`crate::latency`]) under the per-(node, record, subkey) locks and the
//! node's DHT cap before its effect lands.

use std::collections::BTreeSet;
use std::sync::Arc;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rekindle_crypto::keypair_text::keypair_text_from_signing;
use rekindle_records::port::{
    DhtBackend, DhtCallError, DhtErrorKind, RecordDescriptor, RecordReport, RecordSchema,
    RecordValue, ReportScope, WriterKey,
};
use rekindle_utils::RandomSource;

use super::{
    admits, new_key_text, public_text, reachable, rehydrate, replicas, stores, subkey_count,
    watches, writer_public,
};
use crate::clock::SimRandom;
use crate::latency::{self, Slot, Span};
use crate::network::{NetRecord, Network};

fn error(kind: DhtErrorKind, message: impl Into<String>) -> DhtCallError {
    DhtCallError {
        kind,
        message: message.into(),
    }
}

fn try_again() -> DhtCallError {
    error(DhtErrorKind::TryAgain, "offline, try again later")
}

fn not_open(key: &str) -> DhtCallError {
    error(
        DhtErrorKind::InvalidArgument,
        format!("record not open: {key}"),
    )
}

/// One session's view of a node's simulated storage manager.
pub(crate) struct SimDht {
    node: usize,
    net: Arc<Network>,
    rng: Arc<SimRandom>,
}

/// Whether `node`'s DHT is off: no connection, or attached with the public
/// internet not ready (`storage_manager/mod.rs:887-898`: Veilid's
/// `is_online` follows the routing domain's readiness).
fn offline(faults: &crate::network::Faults, node: usize) -> bool {
    faults.dht_offline.contains(&node) || faults.not_ready.contains(&node)
}

impl SimDht {
    pub(crate) fn new(node: usize, net: Arc<Network>, rng: Arc<SimRandom>) -> Self {
        Self { node, net, rng }
    }

    fn offline(&self) -> bool {
        offline(&self.net.inner.lock().faults, self.node)
    }

    /// A network call's latency and slot: `TryAgain` at once when the node
    /// is offline (Veilid checks `dht_is_online()` before any fanout), and
    /// after the latency when the node went offline meanwhile or a "try
    /// again" fault answers (a fanout that failed ran to its end).
    async fn network(&self, span: Span, key: &str, subkeys: &[u32]) -> Result<Slot, DhtCallError> {
        if self.offline() {
            return Err(try_again());
        }
        let slot = self.net.latency.dht(self.node, span, key, subkeys).await;
        let mut inner = self.net.inner.lock();
        if offline(&inner.faults, self.node) {
            return Err(try_again());
        }
        if let Some(left) = inner.faults.try_again.get_mut(&self.node) {
            if *left > 0 {
                *left -= 1;
                return Err(try_again());
            }
        }
        Ok(slot)
    }

    /// Whether a replica the node reaches holds a value at any of
    /// `subkeys` (all when `None`).
    fn held(&self, key: &str, subkeys: Option<&[u32]>) -> bool {
        let inner = self.net.inner.lock();
        inner.records.get(key).is_some_and(|r| {
            let reach = reachable(&r.replicas, &inner.faults.partitions, self.node);
            replicas::holds(&r.replicas, &reach, subkeys)
        })
    }

    /// A read's latency: [`latency::GET`] when a reached replica holds a
    /// value at any of `subkeys` (all when `None`), else the fanout's
    /// [`latency::DEADLINE`].
    fn read_span(&self, key: &str, subkeys: Option<&[u32]>) -> Span {
        if self.held(key, subkeys) {
            latency::GET
        } else {
            latency::DEADLINE
        }
    }

    /// The writer the record is open with, or "record not open".
    fn opened(&self, key: &str) -> Result<Option<String>, DhtCallError> {
        self.net.inner.lock().nodes[self.node]
            .opened
            .get(key)
            .cloned()
            .ok_or_else(|| not_open(key))
    }

    fn descriptor(&self, key: &str) -> Result<RecordDescriptor, DhtCallError> {
        let inner = self.net.inner.lock();
        let record = inner
            .records
            .get(key)
            .ok_or_else(|| error(DhtErrorKind::KeyNotFound, key.to_string()))?;
        let owner = if record.creator == self.node {
            inner.owners.get(key).map(|keypair| WriterKey {
                keypair: keypair.clone(),
                public: public_text(&record.owner),
            })
        } else {
            None
        };
        Ok(RecordDescriptor {
            key: key.to_string(),
            owner,
            subkey_count: subkey_count(&record.schema),
        })
    }

    /// A get over the network: the newest value the reached replicas hold,
    /// stored locally when its seq moved; the local copy after.
    fn refresh(&self, key: &str, subkey: u32) -> Option<RecordValue> {
        let mut inner = self.net.inner.lock();
        let local = inner.nodes[self.node]
            .local
            .get(&(key.to_string(), subkey))
            .cloned();
        let reach = inner.records.get(key).map_or_else(Vec::new, |r| {
            reachable(&r.replicas, &inner.faults.partitions, self.node)
        });
        stores::touch(&mut inner, key, &reach);
        let found = inner
            .records
            .get(key)
            .and_then(|r| replicas::get(&r.replicas, &reach, subkey as usize, local.as_ref()));
        if let Some(found) = &found {
            if local.as_ref().is_none_or(|l| l.seq != found.seq) {
                inner.nodes[self.node]
                    .local
                    .insert((key.to_string(), subkey), found.clone());
            }
        }
        found
    }

    /// Check a write against the record's schema (`set_value.rs:236-242`).
    fn validate(&self, key: &str, subkey: u32, public: &[u8; 32]) -> Result<(), DhtCallError> {
        let inner = self.net.inner.lock();
        let record = inner
            .records
            .get(key)
            .ok_or_else(|| error(DhtErrorKind::Generic, "must have a descriptor"))?;
        if subkey >= subkey_count(&record.schema) || !admits(record, subkey, public) {
            return Err(error(
                DhtErrorKind::Generic,
                format!("failed schema validation: {key}:{subkey}"),
            ));
        }
        Ok(())
    }

    /// The write's fanout over the reached replicas: `Some` is the value
    /// the fanout ended on when it is not ours, now the local copy.
    fn land(
        &self,
        key: &str,
        subkey: u32,
        data: Vec<u8>,
        public: &[u8; 32],
    ) -> Option<RecordValue> {
        let writer = public_text(public);
        let mut guard = self.net.inner.lock();
        let inner = &mut *guard;
        let local = inner.nodes[self.node]
            .local
            .get(&(key.to_string(), subkey))
            .cloned();
        let ours = match local {
            // Same data and writer: the seq stays and the copy is re-pushed
            // (`set_value.rs:205-222`).
            Some(l) if l.data == data && l.writer == writer => l,
            local => RecordValue {
                seq: local.map_or(0, |l| l.seq.saturating_add(1)),
                data,
                writer,
            },
        };
        let record = inner.records.get_mut(key)?;
        let reach = reachable(&record.replicas, &inner.faults.partitions, self.node);
        let fanout = replicas::set(&mut record.replicas, &reach, subkey as usize, &ours);
        watches::mark(record, &fanout.stored, subkey, self.node);
        stores::touch(inner, key, &reach);
        stores::admit(inner, key, &fanout.stored);
        let answer = fanout.consensus.then(|| {
            inner.nodes[self.node]
                .local
                .insert((key.to_string(), subkey), fanout.value.clone());
            fanout.value.clone()
        });
        drop(guard);
        answer.filter(|value| *value != ours)
    }
}

#[async_trait::async_trait]
impl DhtBackend for SimDht {
    fn parse_writer(&self, text: &str) -> Result<WriterKey, DhtCallError> {
        let public =
            writer_public(text).map_err(|e| error(DhtErrorKind::ParseError, e.to_string()))?;
        Ok(WriterKey {
            keypair: text.to_string(),
            public: public_text(&public),
        })
    }

    fn check_key(&self, key: &str) -> Result<String, DhtCallError> {
        let bytes = key
            .strip_prefix("VLD0:")
            .and_then(|b| URL_SAFE_NO_PAD.decode(b).ok())
            .filter(|b| b.len() == 32)
            .ok_or_else(|| error(DhtErrorKind::ParseError, format!("record key: {key}")))?;
        Ok(format!("VLD0:{}", URL_SAFE_NO_PAD.encode(bytes)))
    }

    async fn create(
        &self,
        schema: &RecordSchema,
        owner: Option<&WriterKey>,
    ) -> Result<RecordDescriptor, DhtCallError> {
        // Local: no fanout and no online check (`create_record.rs`).
        let _slot = self
            .net
            .latency
            .dht(self.node, latency::CREATE, "", &[])
            .await;
        super::validate_schema(schema)
            .map_err(|m| error(DhtErrorKind::InvalidArgument, format!("schema: {m}")))?;
        let count = schema.subkey_count();
        let owner = owner.map_or_else(
            || {
                let mut secret = [0u8; 32];
                self.rng.fill(&mut secret);
                keypair_text_from_signing(&rekindle_secrets::ed25519_dalek::SigningKey::from_bytes(
                    &secret,
                ))
            },
            |w| w.keypair.clone(),
        );
        let public =
            writer_public(&owner).map_err(|e| error(DhtErrorKind::ParseError, e.to_string()))?;
        let key = new_key_text(self.rng.as_ref());
        {
            let mut inner = self.net.inner.lock();
            let nodes = inner.nodes.len();
            inner.records.insert(
                key.clone(),
                NetRecord {
                    schema: schema.clone(),
                    owner: public,
                    creator: self.node,
                    replicas: replicas::place(&key, nodes, count),
                },
            );
            inner.owners.insert(key.clone(), owner.clone());
            let slot = &mut inner.nodes[self.node];
            slot.known.insert(key.clone());
            slot.opened.insert(key.clone(), Some(owner));
        }
        self.descriptor(&key)
    }

    async fn open(
        &self,
        key: &str,
        writer: Option<&WriterKey>,
    ) -> Result<RecordDescriptor, DhtCallError> {
        let writer = writer.map(|w| w.keypair.clone());
        let known = self.net.inner.lock().nodes[self.node].known.contains(key);
        if !known {
            // Inspect subkey 0 on the network for the descriptor.
            let span = self.read_span(key, Some(&[0]));
            let _slot = self.network(span, key, &[]).await?;
            if !self.held(key, None) {
                return Err(error(DhtErrorKind::KeyNotFound, key.to_string()));
            }
        }
        {
            let mut inner = self.net.inner.lock();
            if known {
                // A record in the local store: Veilid checks the network
                // against it in the background (`open_record.rs:31-38`).
                rehydrate::request(&mut inner, self.node, key);
            }
            let slot = &mut inner.nodes[self.node];
            slot.known.insert(key.to_string());
            slot.opened.insert(key.to_string(), writer);
        }
        self.descriptor(key)
    }

    async fn close(&self, key: &str) -> Result<(), DhtCallError> {
        let mut inner = self.net.inner.lock();
        let slot = &mut inner.nodes[self.node];
        if let Some(watch) = slot.watches.get_mut(key) {
            *watch = None;
        }
        slot.opened
            .remove(key)
            .map(drop)
            .ok_or_else(|| error(DhtErrorKind::KeyNotFound, key.to_string()))
    }

    async fn get(
        &self,
        key: &str,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<RecordValue>, DhtCallError> {
        self.opened(key)?;
        let local = self.net.inner.lock().nodes[self.node]
            .local
            .get(&(key.to_string(), subkey))
            .cloned();
        if !force_refresh && local.is_some() {
            return Ok(local);
        }
        let span = self.read_span(key, Some(&[subkey]));
        let _slot = self.network(span, key, &[subkey]).await?;
        Ok(self.refresh(key, subkey))
    }

    async fn set(
        &self,
        key: &str,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<&WriterKey>,
    ) -> Result<Option<RecordValue>, DhtCallError> {
        let opened = self.opened(key)?;
        let writer = writer
            .map(|w| w.keypair.clone())
            .or(opened)
            .ok_or_else(|| error(DhtErrorKind::Generic, "value is not writable"))?;
        let public =
            writer_public(&writer).map_err(|e| error(DhtErrorKind::ParseError, e.to_string()))?;
        self.validate(key, subkey, &public)?;
        // In flight until it returns (`inspect_record.rs:169-171`).
        let _setting = super::Setting::new(&self.net, self.node, key, subkey);
        let _slot = self.network(latency::SET, key, &[subkey]).await?;
        Ok(self.land(key, subkey, data, &public))
    }

    async fn inspect(
        &self,
        key: &str,
        subkeys: Option<&[u32]>,
        scope: ReportScope,
    ) -> Result<RecordReport, DhtCallError> {
        self.opened(key)?;
        let _slot = if scope == ReportScope::Local {
            None
        } else {
            Some(self.network(self.read_span(key, subkeys), key, &[]).await?)
        };
        let mut inner = self.net.inner.lock();
        if scope != ReportScope::Local {
            let reach = inner.records.get(key).map_or_else(Vec::new, |r| {
                reachable(&r.replicas, &inner.faults.partitions, self.node)
            });
            stores::touch(&mut inner, key, &reach);
        }
        let record = inner
            .records
            .get(key)
            .ok_or_else(|| error(DhtErrorKind::KeyNotFound, key.to_string()))?;
        let subkeys: Vec<u32> = match subkeys {
            Some(s) => s.to_vec(),
            None => (0..subkey_count(&record.schema)).collect(),
        };
        let slot = &inner.nodes[self.node];
        // UpdateSet simulates a set: each local seq + 1
        // (`inspect_record.rs:208-215`).
        let bump = u32::from(scope == ReportScope::UpdateSet);
        let local_seqs: Vec<Option<u32>> = subkeys
            .iter()
            .map(|s| {
                slot.local
                    .get(&(key.to_string(), *s))
                    .map(|v| v.seq.saturating_add(bump))
            })
            .collect();
        let reach = if scope == ReportScope::Local {
            Vec::new()
        } else {
            reachable(&record.replicas, &inner.faults.partitions, self.node)
        };
        // The fanout starts from the local result for the Update scopes,
        // from nothing for the Sync ones (`:217-228`); Local reports none.
        let from_local = matches!(scope, ReportScope::UpdateGet | ReportScope::UpdateSet);
        let network_seqs = subkeys
            .iter()
            .zip(&local_seqs)
            .map(|(s, l)| match scope {
                ReportScope::Local => None,
                _ => replicas::newest_seq(&record.replicas, &reach, *s as usize)
                    .max(l.filter(|_| from_local)),
            })
            .collect();
        // Its own offline queue, queued or in flight, and the sets in
        // flight (`inspect_record.rs:174-181`).
        let offline_subkeys = subkeys
            .iter()
            .copied()
            .filter(|s| {
                let at = (key.to_string(), *s);
                slot.pushes.contains_key(&at) || slot.setting.contains_key(&at)
            })
            .collect();
        Ok(RecordReport {
            subkeys,
            local_seqs,
            network_seqs,
            offline_subkeys,
        })
    }

    async fn watch(
        &self,
        key: &str,
        subkeys: Option<&[u32]>,
        expiration_us: Option<u64>,
        count: Option<u32>,
    ) -> Result<bool, DhtCallError> {
        let writer = self.opened(key)?;
        // Checked on the node's own clock (`watch_value.rs:224-300`).
        if watches::too_soon(expiration_us, self.net.node_now_us(self.node)) {
            return Err(error(
                DhtErrorKind::InvalidArgument,
                format!("expiration is too soon: {key}"),
            ));
        }
        if !self.offline() {
            drop(
                self.net
                    .latency
                    .dht(self.node, latency::WATCH, key, &[])
                    .await,
            );
        }
        let now = self.net.now_us();
        let mut inner = self.net.inner.lock();
        let Some(record) = inner.records.get(key) else {
            return Err(error(DhtErrorKind::Generic, "no local record found"));
        };
        let all = subkey_count(&record.schema);
        let wanted: BTreeSet<u32> = match subkeys {
            Some(s) if !s.is_empty() => s.iter().copied().filter(|s| *s < all).collect(),
            _ => (0..all).collect(),
        };
        let count = count.unwrap_or(u32::MAX);
        let desired = (count > 0).then(|| watches::Desired {
            subkeys: wanted,
            expiration: expiration_us.filter(|e| *e != 0),
            count,
            member: watches::member_of(record, writer.as_deref()),
        });
        let active = desired.is_some();
        watches::desire(&mut inner, self.node, key, desired, now);
        Ok(active)
    }

    async fn cancel_watch(&self, key: &str, subkeys: Option<&[u32]>) -> Result<bool, DhtCallError> {
        self.opened(key)?;
        let now = self.net.now_us();
        let mut inner = self.net.inner.lock();
        let Some(Some(mut desired)) = inner.nodes[self.node].watches.get(key).cloned() else {
            return Ok(false);
        };
        if let Some(s) = subkeys.filter(|s| !s.is_empty()) {
            for subkey in s {
                desired.subkeys.remove(subkey);
            }
        } else {
            desired.subkeys.clear();
        }
        let remains = !desired.subkeys.is_empty();
        watches::desire(&mut inner, self.node, key, remains.then_some(desired), now);
        Ok(remains)
    }
}

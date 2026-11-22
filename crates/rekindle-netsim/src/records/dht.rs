//! `SimDht`: the netsim's [`DhtBackend`] (plan E2.B slice S3), one per
//! session pool, over the node's side of the simulated storage manager.
//!
//! The network side is the record's replicas (S4, [`super::replicas`]).
//! Each call follows the pinned veilid-core (`~/veilid-wt-058/veilid-core/src/
//! storage_manager`) where the pool depends on it:
//!
//! - every call but create needs the record open on the node, else
//!   `InvalidArgument` "record not open" (`get_value.rs:150`,
//!   `set_value.rs:198-200`, `inspect_record.rs:215`, `watch_value.rs:150`);
//! - an open of a record in the node's local store is local (0.5.8
//!   `open_record.rs:54-81`); any other open needs the network (`TryAgain`
//!   offline, `:84-86`), stops at the first node with the descriptor
//!   (latency [`latency::GET`], `:89-106`), answers `TryAgain` when no
//!   replica is reached and `KeyNotFound` when the reached ones hold
//!   nothing (`:108-119`); a re-open replaces the writer;
//! - a get without `force_refresh` answers the local copy when there is one
//!   (`get_value.rs:162-168`), else asks the network, `TryAgain` offline
//!   (`:169-172`);
//! - a set signs with the given writer or the open's, else `Generic` "value
//!   is not writable"; a writer the schema refuses is `Generic` "failed
//!   schema validation"; the same data and writer resend the local copy,
//!   anything else is local seq + 1 (0.5.8 `set_value.rs:353-366`); offline
//!   is `TryAgain` and stores nothing; how the fanout settles, lands and
//!   what it returns is [`super::fanout`];
//! - a get over the network settles on the replicas' tally and keeps the
//!   local copy unless the settled value replaces it ([`super::replicas`],
//!   0.5.8 `get_value.rs:600-649`);
//! - an inspect in the local scope never leaves the node; any other is
//!   `TryAgain` offline (`inspect_record.rs:312-313`) or with no replica
//!   reached (0.5.8 `inspect_record.rs:346-349`); UpdateSet reports
//!   each local seq + 1, and the Update scopes' network seqs start from the
//!   local result, the Sync ones' from nothing (`:316-332`), then take the
//!   newest seq of the replicas the node reaches; `offline_subkeys` is the
//!   node's own push queue plus its sets in flight (`:284-290`);
//! - a create checks the schema as Veilid ([`super::validate_schema`]); a
//!   watch's expiration is absolute: too soon on the node's own clock (its
//!   skew), clamped by each replica on true time; a transaction get is
//!   [`transaction`];
//! - an open of a locally held record may queue a rehydration
//!   ([`super::rehydrate`]'s `on_open`); a call that reads or writes replicas uses the
//!   record on their stores ([`super::stores`]);
//! - a watch records the wanted state on the node, answers whether one is
//!   wanted and is held by the replicas the node reaches; a close is
//!   node-wide and ends the record's watch ([`super::watches`]).
//!
//! Every call that reaches the network costs Veilid's latency
//! ([`crate::latency`]) under the per-(node, record, subkey) locks and the
//! node's DHT cap before its effect lands.

mod reads;
pub(crate) mod transaction;
mod write;

use std::{collections::BTreeSet, sync::Arc};

use rekindle_crypto::keypair_text::keypair_text_from_signing;
use rekindle_records::port::{
    DhtBackend, DhtCallError, DhtErrorKind, OpenIntent, RecordDescriptor, RecordReport,
    RecordSchema, RecordValue, ReportScope, WriterKey,
};
use rekindle_utils::RandomSource;

use super::{
    admits, canonical_key_text, new_encryption_key, public_text, reachable, record_key_text,
    rehydrate, replicas, size, stores, subkey_count, watches, writer_public,
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

/// A fanout no node answered proves nothing: `TryAgain` (0.5.8
/// `open_record.rs:107-110`, `inspect_record.rs:346-349`). In the model a
/// reachable replica always answers, so "no answer" is "none reached".
fn no_answer(call: &str) -> DhtCallError {
    error(
        DhtErrorKind::TryAgain,
        format!("no node answered the {call}"),
    )
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
/// internet not ready (0.5.8 `storage_manager/mod.rs:906-908,1097-1103`: Veilid's
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
        let asked_offline = self
            .net
            .inner
            .lock()
            .faults
            .detector_offline
            .contains(&self.node);
        let slot = self.net.latency.dht(self.node, span, key, subkeys).await;
        let mut inner = self.net.inner.lock();
        if offline(&inner.faults, self.node) {
            return Err(try_again());
        }
        // A replica's answer to a question sent while the online detector
        // said Offline proves the network: the detector leaves Offline and
        // the node's deferred loopback failures count again (0.5.8
        // `rpc_processor/mod.rs:1711-1728` `record_answer_received`,
        // `online_detector.rs:1343-1347`; [`crate::loopback`]).
        let answered = inner
            .records
            .get(key)
            .is_some_and(|r| !reachable(&r.replicas, &inner.faults, self.node).is_empty());
        if asked_offline && answered {
            inner.faults.detector_offline.remove(&self.node);
        }
        drop(inner);
        if self.fault_try_again() {
            return Err(try_again());
        }
        Ok(slot)
    }

    /// Whether a "try again" fault answers this call (one fewer left).
    fn fault_try_again(&self) -> bool {
        let mut inner = self.net.inner.lock();
        match inner.faults.try_again.get_mut(&self.node) {
            Some(left) if *left > 0 => {
                *left -= 1;
                true
            }
            _ => false,
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
            owner_public: public_text(&record.owner),
            subkey_count: subkey_count(&record.schema),
        })
    }

    /// Check a write against the record's schema, its writer and its size
    /// (`set_value.rs:396-405`, [`super::size`]).
    fn validate(
        &self,
        key: &str,
        subkey: u32,
        public: &[u8; 32],
        len: usize,
    ) -> Result<(), DhtCallError> {
        let inner = self.net.inner.lock();
        let record = inner
            .records
            .get(key)
            .ok_or_else(|| error(DhtErrorKind::Generic, "must have a descriptor"))?;
        if subkey >= subkey_count(&record.schema)
            || !admits(record, subkey, public)
            || len > size::max_value_len(&record.schema)
        {
            return Err(error(
                DhtErrorKind::Generic,
                format!("failed schema validation: {key}:{subkey}"),
            ));
        }
        Ok(())
    }
}

impl SimDht {
    /// An open with Veilid's `rehydrate` option
    /// (`OpenDHTRecordOptions`, 0.5.8 `open_record.rs:62-81`).
    pub(crate) async fn open_with(
        &self,
        key: &str,
        writer: Option<&WriterKey>,
        rehydrate: bool,
    ) -> Result<RecordDescriptor, DhtCallError> {
        let writer = writer.map(|w| w.keypair.clone());
        let known = self.net.inner.lock().nodes[self.node].known.contains(key);
        if !known {
            // Inspect subkey 0 for the descriptor, stopping at the first
            // node that has it (0.5.8 `open_record.rs:90-105`); not found needs a
            // node that said so (`:107-110`).
            let _slot = self.network(latency::GET, key, &[]).await?;
            if self.reach(key).is_empty() {
                return Err(no_answer("open"));
            }
            if !self.held(key, None) {
                return Err(error(DhtErrorKind::KeyNotFound, key.to_string()));
            }
        }
        {
            let mut inner = self.net.inner.lock();
            if known {
                rehydrate::on_open(&mut inner, self.node, key, rehydrate);
            }
            let slot = &mut inner.nodes[self.node];
            slot.known.insert(key.to_string());
            slot.opened.insert(key.to_string(), writer);
        }
        self.descriptor(key)
    }

    /// The replicas this node reaches for `key`.
    fn reach(&self, key: &str) -> Vec<usize> {
        let inner = self.net.inner.lock();
        inner.records.get(key).map_or_else(Vec::new, |r| {
            reachable(&r.replicas, &inner.faults, self.node)
        })
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
        canonical_key_text(key)
            .ok_or_else(|| error(DhtErrorKind::ParseError, format!("record key: {key}")))
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
        // The encryption key after the owner, as 0.5.8 `create_record.rs:82-93`.
        let encryption_key = new_encryption_key(self.rng.as_ref());
        let key = record_key_text(&public, schema, &encryption_key);
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
            slot.created.insert(key.clone());
            slot.opened.insert(key.clone(), Some(owner));
        }
        self.descriptor(&key)
    }

    async fn open(
        &self,
        key: &str,
        writer: Option<&WriterKey>,
        intent: OpenIntent,
    ) -> Result<RecordDescriptor, DhtCallError> {
        // As the production backend maps it (`rekindle-protocol`
        // `VeilidDht::open`): Hold asks for rehydration, Check never does.
        self.open_with(key, writer, intent.rehydrates()).await
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
        self.net.count_app_get(self.node, key);
        let span = self.read_span(key, Some(&[subkey]));
        let _slot = self.network(span, key, &[subkey]).await?;
        let mut inner = self.net.inner.lock();
        Ok(replicas::refresh(
            &mut inner,
            self.node,
            key,
            subkey,
            force_refresh,
        ))
    }

    async fn get_newest(
        &self,
        key: &str,
        subkey: u32,
    ) -> Result<Option<RecordValue>, DhtCallError> {
        self.transaction_get(key, subkey).await
    }

    async fn set(
        &self,
        key: &str,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<&WriterKey>,
    ) -> Result<Option<RecordValue>, DhtCallError> {
        self.write(key, subkey, data, writer, None).await
    }

    async fn set_above(
        &self,
        key: &str,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<&WriterKey>,
        min_seq: u32,
    ) -> Result<Option<RecordValue>, DhtCallError> {
        self.write(key, subkey, data, writer, Some(min_seq)).await
    }

    async fn inspect(
        &self,
        key: &str,
        subkeys: Option<&[u32]>,
        scope: ReportScope,
    ) -> Result<RecordReport, DhtCallError> {
        self.net.count_app_inspect(self.node, key);
        self.opened(key)?;
        let _slot = if scope == ReportScope::Local {
            None
        } else {
            Some(self.network(self.read_span(key, subkeys), key, &[]).await?)
        };
        if scope != ReportScope::Local && self.reach(key).is_empty() {
            return Err(no_answer("inspect"));
        }
        let mut inner = self.net.inner.lock();
        if scope != ReportScope::Local {
            let reach = inner.records.get(key).map_or_else(Vec::new, |r| {
                reachable(&r.replicas, &inner.faults, self.node)
            });
            stores::touch(&mut inner, key, &reach);
        }
        let record = inner
            .records
            .get(key)
            .ok_or_else(|| error(DhtErrorKind::KeyNotFound, key.to_string()))?;
        let subkeys: Vec<u32> = match subkeys {
            // Truncated to the schema, as a sorted range set
            // (`inspect_record.rs:20`).
            Some(s) => {
                let all = subkey_count(&record.schema);
                let mut s: Vec<u32> = s.iter().copied().filter(|s| *s < all).collect();
                s.sort_unstable();
                s.dedup();
                s
            }
            None => (0..subkey_count(&record.schema)).collect(),
        };
        let slot = &inner.nodes[self.node];
        // UpdateSet simulates a set: each local seq + 1
        // (0.5.8 `inspect_record.rs:316-321`).
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
            reachable(&record.replicas, &inner.faults, self.node)
        };
        // The fanout starts from the local result for the Update scopes,
        // from nothing for the Sync ones (`:328-332`); Local reports none.
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
        // flight (0.5.8 `inspect_record.rs:284-290`).
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
        // Checked on the node's own clock (`watch_value.rs:290-363`).
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

//! `SimDht`: the netsim's [`DhtBackend`] (plan E2.B slice S3), one per
//! session pool, over the node's side of the simulated storage manager.
//!
//! Today's model keeps one network value per subkey (replicas come in S4).
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
//!   `local_record_store_interface.rs:61-62`); a newer network value is
//!   returned and stored locally (`set_value.rs:620-645`);
//! - an inspect in the local scope never leaves the node; any other is
//!   `TryAgain` offline (`inspect_record.rs:205-206`); with
//!   `allow_offline` false no write is ever queued, so `offline_subkeys`
//!   is empty;
//! - a watch records the wanted state on the node and answers `true`
//!   (accepted locally); a close is node-wide and ends the record's watch.
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

use super::{admits, new_key_text, public_text, subkey_count, writer_public};
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

impl SimDht {
    pub(crate) fn new(node: usize, net: Arc<Network>, rng: Arc<SimRandom>) -> Self {
        Self { node, net, rng }
    }

    fn offline(&self) -> bool {
        self.net
            .inner
            .lock()
            .faults
            .dht_offline
            .contains(&self.node)
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
        if inner.faults.dht_offline.contains(&self.node) {
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

    /// A read's latency: [`latency::GET`] when the network holds a value at
    /// any of `subkeys` (all when `None`), else the fanout's
    /// [`latency::DEADLINE`].
    fn read_span(&self, key: &str, subkeys: Option<&[u32]>) -> Span {
        let inner = self.net.inner.lock();
        let held = inner.records.get(key).is_some_and(|r| match subkeys {
            Some(s) => s
                .iter()
                .any(|s| r.values.get(*s as usize).is_some_and(Option::is_some)),
            None => r.values.iter().any(Option::is_some),
        });
        if held {
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

    /// Refresh the local copy of `subkey` from the network when the network
    /// is newer; the local copy after.
    fn refresh(&self, key: &str, subkey: u32) -> Option<RecordValue> {
        let mut inner = self.net.inner.lock();
        let network = inner
            .records
            .get(key)
            .and_then(|r| r.values.get(subkey as usize).cloned().flatten());
        let slot = &mut inner.nodes[self.node];
        let local = slot.local.get(&(key.to_string(), subkey)).cloned();
        match (network, local) {
            (Some(n), l) if l.as_ref().is_none_or(|l| n.seq > l.seq) => {
                slot.local.insert((key.to_string(), subkey), n.clone());
                Some(n)
            }
            (_, local) => local,
        }
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

    /// The write's effect once its fanout ran: `Some` is a newer network
    /// value, now the local copy.
    fn land(
        &self,
        key: &str,
        subkey: u32,
        data: Vec<u8>,
        public: &[u8; 32],
    ) -> Option<RecordValue> {
        let writer = public_text(public);
        let mut inner = self.net.inner.lock();
        let network = inner
            .records
            .get(key)
            .and_then(|r| r.values.get(subkey as usize).cloned().flatten());
        let slot = &mut inner.nodes[self.node];
        let local = slot.local.get(&(key.to_string(), subkey)).cloned();
        let value = match local {
            // Same data and writer: the seq stays and the copy is re-pushed.
            Some(l) if l.data == data && l.writer == writer => l,
            local => RecordValue {
                seq: local.map_or(0, |l| l.seq.saturating_add(1)),
                data,
                writer,
            },
        };
        if let Some(newer) = network
            .as_ref()
            .filter(|n| n.seq > value.seq || (n.seq == value.seq && n.data != value.data))
        {
            slot.local.insert((key.to_string(), subkey), newer.clone());
            return Some(newer.clone());
        }
        slot.local.insert((key.to_string(), subkey), value.clone());
        let behind = network.is_none_or(|n| n.seq < value.seq);
        if let Some(r) = inner.records.get_mut(key) {
            r.published = true;
            if behind {
                r.values[subkey as usize] = Some(value.clone());
            }
        }
        drop(inner);
        if behind {
            self.net.notify_watchers(self.node, key, subkey, &value);
        }
        None
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
        let count = schema.subkey_count();
        if count == 0 || count > 1024 {
            return Err(error(
                DhtErrorKind::InvalidArgument,
                format!("schema: {count} subkeys"),
            ));
        }
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
            inner.records.insert(
                key.clone(),
                NetRecord {
                    schema: schema.clone(),
                    owner: public,
                    creator: self.node,
                    published: false,
                    values: vec![None; count],
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
            let published = self
                .net
                .inner
                .lock()
                .records
                .get(key)
                .is_some_and(|r| r.published);
            if !published {
                return Err(error(DhtErrorKind::KeyNotFound, key.to_string()));
            }
        }
        {
            let mut inner = self.net.inner.lock();
            let slot = &mut inner.nodes[self.node];
            slot.known.insert(key.to_string());
            slot.opened.insert(key.to_string(), writer);
        }
        self.descriptor(key)
    }

    async fn close(&self, key: &str) -> Result<(), DhtCallError> {
        let mut inner = self.net.inner.lock();
        let slot = &mut inner.nodes[self.node];
        slot.watches.remove(key);
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
        if self
            .net
            .inner
            .lock()
            .faults
            .stale_reads
            .contains(&self.node)
        {
            return Ok(local);
        }
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
        let inner = self.net.inner.lock();
        let record = inner
            .records
            .get(key)
            .ok_or_else(|| error(DhtErrorKind::KeyNotFound, key.to_string()))?;
        let subkeys: Vec<u32> = match subkeys {
            Some(s) => s.to_vec(),
            None => (0..subkey_count(&record.schema)).collect(),
        };
        let local = &inner.nodes[self.node].local;
        let local_seqs = subkeys
            .iter()
            .map(|s| local.get(&(key.to_string(), *s)).map(|v| v.seq))
            .collect();
        let network = scope != ReportScope::Local && !inner.faults.stale_reads.contains(&self.node);
        let network_seqs = subkeys
            .iter()
            .map(|s| {
                network
                    .then(|| record.values.get(*s as usize).cloned().flatten())
                    .flatten()
                    .map(|v| v.seq)
            })
            .collect();
        Ok(RecordReport {
            subkeys,
            local_seqs,
            network_seqs,
            offline_subkeys: Vec::new(),
        })
    }

    async fn watch(
        &self,
        key: &str,
        subkeys: Option<&[u32]>,
        _expiration_us: Option<u64>,
        _count: Option<u32>,
    ) -> Result<bool, DhtCallError> {
        self.opened(key)?;
        if !self.offline() {
            drop(
                self.net
                    .latency
                    .dht(self.node, latency::WATCH, key, &[])
                    .await,
            );
        }
        let mut inner = self.net.inner.lock();
        let count = inner
            .records
            .get(key)
            .map_or(0, |r| subkey_count(&r.schema));
        let wanted: BTreeSet<u32> = match subkeys {
            Some(s) => s.iter().copied().collect(),
            None => (0..count).collect(),
        };
        inner.nodes[self.node]
            .watches
            .insert(key.to_string(), wanted);
        Ok(true)
    }

    async fn cancel_watch(&self, key: &str, subkeys: Option<&[u32]>) -> Result<bool, DhtCallError> {
        self.opened(key)?;
        let mut inner = self.net.inner.lock();
        let watches = &mut inner.nodes[self.node].watches;
        let remains = match (subkeys, watches.get_mut(key)) {
            (Some(s), Some(watched)) => {
                for subkey in s {
                    watched.remove(subkey);
                }
                !watched.is_empty()
            }
            _ => false,
        };
        if !remains {
            watches.remove(key);
        }
        Ok(remains)
    }
}

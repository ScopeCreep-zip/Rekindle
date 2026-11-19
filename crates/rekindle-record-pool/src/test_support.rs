//! An in-memory [`DhtBackend`] for tests that drive production code through
//! a real [`RecordPool`] (feature `test-support`). It stores what it is
//! given, keeps the seq on an identical rewrite as Veilid does
//! (`set_value.rs:353-366`, 0.5.8), and can answer a set the way Veilid
//! 0.5.8 does when a subkey is contested (`set_value.rs:753-760`): with the
//! value that settled, ours ([`Contest::Ours`]) or a rival's
//! ([`Contest::Rival`]). The netsim is the faithful network; this is one
//! node's store with the set result made explicit.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use rekindle_lifecycle::SessionScope;
use rekindle_records::port::{
    DhtBackend, DhtCallError, DhtErrorKind, OpenIntent, RecordDescriptor, RecordReport,
    RecordSchema, RecordValue, ReportScope, WriterKey,
};

use crate::{RecordCloser, RecordPool};

/// Subkeys of a record opened by a key this backend never created.
pub const UNKNOWN_RECORD_SUBKEYS: u32 = 16;

/// What a set returns besides storing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Contest {
    /// Settled on ours with no rival: `Ok(None)`.
    #[default]
    None,
    /// Ours settled, and some node holds a rival at our seq: `Ok(Some(ours))`.
    Ours,
    /// A rival at our seq won the tally: stored locally and returned.
    Rival { data: Vec<u8>, writer: String },
}

struct MemRecord {
    subkeys: u32,
    owner: WriterKey,
    writer: Option<WriterKey>,
    values: HashMap<u32, RecordValue>,
}

#[derive(Default)]
struct MemState {
    records: HashMap<String, MemRecord>,
    created: u32,
    /// Every open, with its intent, in order.
    opens: Vec<(String, OpenIntent)>,
    /// Every get, as `(key, subkey, force_refresh)`, in order.
    gets: Vec<(String, u32, bool)>,
    /// Network seqs an inspect claims with no value behind them (a poisoned
    /// 0.5.7 storage node, threat model D8), by `(key, subkey)`.
    claims: HashMap<(String, u32), u32>,
}

/// One node's in-memory record store. A writer's keypair text is also its
/// public key text.
#[derive(Default)]
pub struct MemDht {
    state: Mutex<MemState>,
    contest: Mutex<Contest>,
    offline: Mutex<bool>,
    /// The pool's readiness sender, kept alive with the backend: the
    /// re-push loop ends when its sender is gone.
    ready: Mutex<Option<tokio::sync::watch::Sender<bool>>>,
}

fn writer_key(text: &str) -> WriterKey {
    WriterKey {
        keypair: text.to_string(),
        public: text.to_string(),
    }
}

fn error(kind: DhtErrorKind, message: &str) -> DhtCallError {
    DhtCallError {
        kind,
        message: message.to_string(),
    }
}

impl MemDht {
    /// How every set from now on answers.
    pub fn contest(&self, contest: Contest) {
        *self.contest.lock() = contest;
    }

    /// Whether sets fail with `TryAgain` (the node is offline).
    pub fn offline(&self, offline: bool) {
        *self.offline.lock() = offline;
    }

    /// Every open so far, with its intent, in order.
    #[must_use]
    pub fn opens(&self) -> Vec<(String, OpenIntent)> {
        self.state.lock().opens.clone()
    }

    /// From now on an inspect of `key`'s `subkey` claims network seq `seq`
    /// (when above what is stored), while a get still returns the stored
    /// value: an inflated seq no value confirms (threat model D8).
    pub fn claim(&self, key: &str, subkey: u32, seq: u32) {
        self.state
            .lock()
            .claims
            .insert((key.to_string(), subkey), seq);
    }

    /// Every get so far, as `(key, subkey, force_refresh)`, in order.
    #[must_use]
    pub fn gets(&self) -> Vec<(String, u32, bool)> {
        self.state.lock().gets.clone()
    }

    /// The stored value of `key`'s `subkey`.
    #[must_use]
    pub fn value(&self, key: &str, subkey: u32) -> Option<RecordValue> {
        self.state
            .lock()
            .records
            .get(key)
            .and_then(|r| r.values.get(&subkey).cloned())
    }

    fn descriptor(key: &str, record: &MemRecord, with_owner: bool) -> RecordDescriptor {
        RecordDescriptor {
            key: key.to_string(),
            owner: with_owner.then(|| record.owner.clone()),
            owner_public: record.owner.public.clone(),
            subkey_count: record.subkeys,
        }
    }
}

#[async_trait::async_trait]
impl DhtBackend for MemDht {
    fn parse_writer(&self, text: &str) -> Result<WriterKey, DhtCallError> {
        Ok(writer_key(text))
    }

    fn check_key(&self, key: &str) -> Result<String, DhtCallError> {
        Ok(key.to_string())
    }

    async fn create(
        &self,
        schema: &RecordSchema,
        owner: Option<&WriterKey>,
    ) -> Result<RecordDescriptor, DhtCallError> {
        let mut state = self.state.lock();
        state.created += 1;
        let key = format!("mem-record-{}", state.created);
        let owner = owner
            .cloned()
            .unwrap_or_else(|| writer_key(&format!("mem-owner-{}", state.created)));
        let subkeys = u32::try_from(schema.subkey_count())
            .map_err(|_| error(DhtErrorKind::InvalidArgument, "too many subkeys"))?;
        let record = MemRecord {
            subkeys,
            owner,
            writer: None,
            values: HashMap::new(),
        };
        let descriptor = Self::descriptor(&key, &record, true);
        state.records.insert(key, record);
        Ok(descriptor)
    }

    async fn open(
        &self,
        key: &str,
        writer: Option<&WriterKey>,
        intent: OpenIntent,
    ) -> Result<RecordDescriptor, DhtCallError> {
        let mut state = self.state.lock();
        state.opens.push((key.to_string(), intent));
        let record = state
            .records
            .entry(key.to_string())
            .or_insert_with(|| MemRecord {
                subkeys: UNKNOWN_RECORD_SUBKEYS,
                owner: writer_key(&format!("{key}-owner")),
                writer: None,
                values: HashMap::new(),
            });
        if writer.is_some() {
            record.writer = writer.cloned();
        }
        Ok(Self::descriptor(key, record, false))
    }

    async fn close(&self, _key: &str) -> Result<(), DhtCallError> {
        Ok(())
    }

    async fn get(
        &self,
        key: &str,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<RecordValue>, DhtCallError> {
        self.state
            .lock()
            .gets
            .push((key.to_string(), subkey, force_refresh));
        Ok(self.value(key, subkey))
    }

    /// One store, so the transaction's newest value is the stored one.
    async fn get_newest(
        &self,
        key: &str,
        subkey: u32,
    ) -> Result<Option<RecordValue>, DhtCallError> {
        Ok(self.value(key, subkey))
    }

    async fn set(
        &self,
        key: &str,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<&WriterKey>,
    ) -> Result<Option<RecordValue>, DhtCallError> {
        if *self.offline.lock() {
            return Err(error(DhtErrorKind::TryAgain, "offline"));
        }
        let contest = self.contest.lock().clone();
        let mut state = self.state.lock();
        let record = state
            .records
            .get_mut(key)
            .ok_or_else(|| error(DhtErrorKind::KeyNotFound, "record not open"))?;
        if subkey >= record.subkeys {
            return Err(error(DhtErrorKind::InvalidArgument, "subkey out of range"));
        }
        let signer = writer
            .or(record.writer.as_ref())
            .unwrap_or(&record.owner)
            .public
            .clone();
        let seq = match record.values.get(&subkey) {
            Some(v) if v.data == data && v.writer == signer => v.seq,
            Some(v) => v.seq + 1,
            None => 0,
        };
        let ours = RecordValue {
            seq,
            data,
            writer: signer,
        };
        let (stored, returned) = match contest {
            Contest::None => (ours, None),
            Contest::Ours => (ours.clone(), Some(ours)),
            Contest::Rival { data, writer } => {
                let rival = RecordValue { seq, data, writer };
                (rival.clone(), Some(rival))
            }
        };
        record.values.insert(subkey, stored);
        Ok(returned)
    }

    async fn inspect(
        &self,
        key: &str,
        subkeys: Option<&[u32]>,
        _scope: ReportScope,
    ) -> Result<RecordReport, DhtCallError> {
        let state = self.state.lock();
        let record = state
            .records
            .get(key)
            .ok_or_else(|| error(DhtErrorKind::KeyNotFound, "record not open"))?;
        let subkeys: Vec<u32> =
            subkeys.map_or_else(|| (0..record.subkeys).collect(), <[u32]>::to_vec);
        let local_seqs: Vec<Option<u32>> = subkeys
            .iter()
            .map(|s| record.values.get(s).map(|v| v.seq))
            .collect();
        let network_seqs = subkeys
            .iter()
            .zip(&local_seqs)
            .map(|(s, local)| {
                let claim = state.claims.get(&(key.to_string(), *s)).copied();
                (*local).max(claim)
            })
            .collect();
        Ok(RecordReport {
            network_seqs,
            local_seqs,
            subkeys,
            offline_subkeys: Vec::new(),
        })
    }

    async fn watch(
        &self,
        _key: &str,
        _subkeys: Option<&[u32]>,
        _expiration_us: Option<u64>,
        _count: Option<u32>,
    ) -> Result<bool, DhtCallError> {
        Ok(true)
    }

    async fn cancel_watch(
        &self,
        _key: &str,
        _subkeys: Option<&[u32]>,
    ) -> Result<bool, DhtCallError> {
        Ok(true)
    }
}

/// A ready pool over `backend`.
#[must_use]
pub fn pool_on(backend: Arc<MemDht>) -> Arc<RecordPool> {
    let (ready_tx, ready) = tokio::sync::watch::channel(true);
    *backend.ready.lock() = Some(ready_tx);
    RecordPool::new(
        backend,
        SessionScope::new("mem pool", Arc::new(|_| {})),
        ready,
        RecordCloser::new(),
    )
}

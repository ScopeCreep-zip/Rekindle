//! An in-memory [`DhtBackend`] for tests that drive production code through
//! a real [`RecordPool`] (feature `test-support`). It stores what it is
//! given, keeps the seq on an identical rewrite as Veilid does
//! (`set_value.rs:353-366`, 0.5.8), and can answer a set the way Veilid
//! 0.5.8 does when a subkey is contested (`set_value.rs:753-760`): with the
//! value that settled, ours ([`Contest::Ours`]) or a rival's
//! ([`Contest::Rival`]). The netsim is the faithful network; this is one
//! node's store with the set result made explicit.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use parking_lot::Mutex;
use rekindle_lifecycle::SessionScope;
use rekindle_records::port::{
    DhtBackend, DhtCallError, DhtErrorKind, OpenIntent, RecordDescriptor, RecordReport,
    RecordSchema, RecordValue, ReportScope, ValueChange, WriterKey,
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
    /// The rival sits at the seq of the first such write of each subkey; a
    /// write at another seq is not contested by it.
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
    /// Values another node wrote that the network holds and our local copy
    /// does not yet, by `(key, subkey)`: a get that reaches the network
    /// stores one locally (0.5.8 `get_value.rs:199-205`).
    remote: HashMap<(String, u32), RecordValue>,
    /// The seq a rival holds under [`Contest::Ours`], by `(key, subkey)`.
    rivals: HashMap<(String, u32), u32>,
    /// Subkeys the network holds none of while the local copy has them (a
    /// write that stored on too few nodes), until a set of them lands.
    unheld: HashSet<(String, u32)>,
    /// How many network-scope inspects were made.
    inspects: u32,
    /// Whether network-scope inspects fail with `TryAgain`.
    inspect_fails: bool,
}

/// One node's in-memory record store. A writer's keypair text is also its
/// public key text.
#[derive(Default)]
pub struct MemDht {
    state: Mutex<MemState>,
    contest: Mutex<Contest>,
    offline: Mutex<bool>,
    below_consensus: Mutex<bool>,
    /// The pool's readiness sender, kept alive with the backend: the
    /// re-push loop ends when its sender is gone.
    ready: Mutex<Option<tokio::sync::watch::Sender<bool>>>,
    /// Every set the backend was asked for.
    sets: Mutex<u32>,
    /// Every change the pool over this backend published (its sink).
    published: Mutex<Vec<ValueChange>>,
}

fn writer_key(text: &str) -> WriterKey {
    WriterKey::new(
        text.to_owned(),
        rekindle_secrets::record_writer::RecordWriterSecret::from_text(text),
    )
}

fn error(kind: DhtErrorKind, message: &str) -> DhtCallError {
    DhtCallError {
        kind,
        message: message.to_string(),
    }
}

impl MemDht {
    /// How many sets the backend was asked for.
    pub fn sets(&self) -> u32 {
        *self.sets.lock()
    }

    /// How every set from now on answers.
    pub fn contest(&self, contest: Contest) {
        *self.contest.lock() = contest;
    }

    /// Whether sets fail with `TryAgain` (the node is offline).
    pub fn offline(&self, offline: bool) {
        *self.offline.lock() = offline;
    }

    /// Whether every set's fanout ends below consensus: nothing is stored
    /// and the set answers `Ok(None)` (fork `e1158844`
    /// `set_value.rs:731-738` → `local_record_store_interface.rs:61-62`,
    /// then `set_value.rs:306-308`).
    pub fn below_consensus(&self, below: bool) {
        *self.below_consensus.lock() = below;
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

    /// From now on an inspect of `key`'s `subkey` reports no network seq,
    /// until the next set of it lands.
    pub fn unhold(&self, key: &str, subkey: u32) {
        self.state.lock().unheld.insert((key.to_string(), subkey));
    }

    /// Whether network-scope inspects fail with `TryAgain` from now on.
    pub fn fail_inspects(&self, fail: bool) {
        self.state.lock().inspect_fails = fail;
    }

    /// How many network-scope inspects were made.
    #[must_use]
    pub fn inspects(&self) -> u32 {
        self.state.lock().inspects
    }

    /// Every get so far, as `(key, subkey, force_refresh)`, in order.
    #[must_use]
    pub fn gets(&self) -> Vec<(String, u32, bool)> {
        self.state.lock().gets.clone()
    }

    /// Another node writes `data` as `writer` to `key`'s `subkey`: the
    /// network holds it at the next seq, our local copy does not.
    pub fn write_remote(&self, key: &str, subkey: u32, data: &[u8], writer: &str) {
        let mut state = self.state.lock();
        let at = (key.to_string(), subkey);
        let local = (state.records.get(key)).and_then(|r| r.values.get(&subkey).map(|v| v.seq));
        let seq = local
            .max(state.remote.get(&at).map(|v| v.seq))
            .map_or(0, |seq| seq + 1);
        state.remote.insert(
            at,
            RecordValue {
                seq,
                data: data.to_vec(),
                writer: writer.to_string(),
            },
        );
    }

    /// Every change the pool over this backend published, in order.
    #[must_use]
    pub fn published(&self) -> Vec<ValueChange> {
        self.published.lock().clone()
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
        let mut guard = self.state.lock();
        let state = &mut *guard;
        state.gets.push((key.to_string(), subkey, force_refresh));
        let at = (key.to_string(), subkey);
        if let Some(record) = state.records.get_mut(key) {
            let local = record.values.get(&subkey).map(|v| v.seq);
            // Only a get that reaches the network (forced, or with no local
            // copy, `get_value.rs:159-171`) stores the network's newer value.
            if force_refresh || local.is_none() {
                if let Some(remote) = state.remote.remove(&at) {
                    if Some(remote.seq) > local {
                        record.values.insert(subkey, remote);
                    }
                }
            }
        }
        drop(guard);
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
        *self.sets.lock() += 1;
        self.write(key, subkey, data, writer, 0)
    }

    async fn set_above(
        &self,
        key: &str,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<&WriterKey>,
        min_seq: u32,
    ) -> Result<Option<RecordValue>, DhtCallError> {
        self.write(key, subkey, data, writer, min_seq)
    }

    async fn inspect(
        &self,
        key: &str,
        subkeys: Option<&[u32]>,
        scope: ReportScope,
    ) -> Result<RecordReport, DhtCallError> {
        let mut state = self.state.lock();
        if scope != ReportScope::Local {
            state.inspects += 1;
            if state.inspect_fails {
                return Err(error(DhtErrorKind::TryAgain, "no node answered"));
            }
        }
        let state = &*state;
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
                let at = (key.to_string(), *s);
                if state.unheld.contains(&at) {
                    return None;
                }
                let claim = state.claims.get(&at).copied();
                let remote = state.remote.get(&at).map(|v| v.seq);
                (*local).max(claim).max(remote)
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
    let sink = Arc::clone(&backend);
    RecordPool::new(
        backend,
        SessionScope::new("mem pool", Arc::new(|_| {})),
        ready,
        RecordCloser::new(),
        Arc::new(move |change| sink.published.lock().push(change)),
    )
}

impl MemDht {
    /// A set at a seq of at least `min_seq` (0 for a plain set).
    fn write(
        &self,
        key: &str,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<&WriterKey>,
        min_seq: u32,
    ) -> Result<Option<RecordValue>, DhtCallError> {
        if *self.offline.lock() {
            return Err(error(DhtErrorKind::TryAgain, "offline"));
        }
        if *self.below_consensus.lock() {
            return Ok(None);
        }
        let contest = self.contest.lock().clone();
        let mut guard = self.state.lock();
        let state = &mut *guard;
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
        }
        // `min_seqnum` raises the seq after the increment, and re-encrypts
        // an identical value it lifts (0.5.8 `set_value.rs:353-386`).
        .max(min_seq);
        let ours = RecordValue {
            seq,
            data,
            writer: signer,
        };
        let (stored, returned) = match contest {
            Contest::None => (ours, None),
            Contest::Ours => {
                let rival = *state.rivals.entry((key.to_string(), subkey)).or_insert(seq);
                let contested = (rival == seq).then(|| ours.clone());
                (ours, contested)
            }
            Contest::Rival { data, writer } => {
                let rival = RecordValue { seq, data, writer };
                (rival.clone(), Some(rival))
            }
        };
        record.values.insert(subkey, stored);
        state.unheld.remove(&(key.to_string(), subkey));
        Ok(returned)
    }
}

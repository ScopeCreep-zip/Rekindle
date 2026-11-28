//! Records: one session's pool of borrowed DHT records, and the store that
//! starts a pool per unlock.

use std::sync::Arc;

use crate::lease::LeaseId;

use super::{OpenIntent, PortError};

/// A stored subkey value: its sequence number, bytes and writer key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordValue {
    pub seq: u32,
    pub data: Vec<u8>,
    /// The writer's public key text.
    pub writer: String,
}

/// What a write did on the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetOutcome {
    /// Stored at consensus (ours settled, possibly over a rival at its
    /// seq, which is only logged).
    Landed,
    /// The write did not reach consensus: the node is offline, or the
    /// fanout ended below consensus, which Veilid also calls offline
    /// (`was_offline`): under `AllowOffline(false)` its offline handler bails
    /// `TryAgain` before writing or queuing (fork `e1158844`
    /// `set_value.rs:710-738` → `local_record_store_interface.rs:61-62`),
    /// which `set_value` returns as `Ok(None)` unless the write closes the
    /// subkey (`set_value.rs:306-308`); the pool reads that `Ok(None)` by the
    /// local seq, which only a landing moves. Nothing was stored locally or queued; the
    /// replicas that took a partial value keep it: a replica writes the
    /// subkey when the set arrives, whatever the fanout ends on, and nothing
    /// takes it back (`inbound_set_value`, `set_value.rs:909-923`).
    Offline,
    /// The write settled on another value (newer, or a rival at our seq
    /// that won the tally), returned here. A settled value whose data and
    /// writer are ours is `Landed` or `Unchanged`, never this.
    Superseded(RecordValue),
    /// The value equals the local copy, which is already ours: Veilid re-sent
    /// the stored bytes at their seq (0.5.8 `set_value.rs:353-366`) and no
    /// other value settled. The local seq did not move, so Veilid's `Ok(None)`
    /// cannot say whether this re-push reached consensus; the local copy was
    /// stored only by an earlier landed write or a network read (a missed
    /// write stores nothing). Never a missed new value: that is `Offline`.
    Unchanged,
}

impl SetOutcome {
    /// The write did not reach consensus, or lost to a newer value.
    #[must_use]
    pub fn missed(&self) -> bool {
        matches!(self, Self::Offline | Self::Superseded(_))
    }

    /// `Ok` unless the write [`missed`](Self::missed).
    ///
    /// # Errors
    /// [`PortError::NotStored`] for a missed write.
    pub fn require_stored(self, subkey: u32) -> Result<(), PortError> {
        if self.missed() {
            return Err(PortError::NotStored {
                subkey,
                outcome: match self {
                    Self::Superseded(_) => "superseded by a newer value".into(),
                    other => format!("{other:?}"),
                },
            });
        }
        Ok(())
    }
}

/// A member's subkeys in an SMPL schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaMember {
    /// The member's raw Ed25519 public key (its member id).
    pub key: [u8; 32],
    pub subkeys: u16,
}

/// A record's schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordSchema {
    /// Owner subkeys only.
    Dflt { owner_subkeys: u16 },
    /// Owner subkeys, then each member's.
    Smpl {
        owner_subkeys: u16,
        members: Vec<SchemaMember>,
    },
}

impl RecordSchema {
    /// Every subkey the schema holds.
    #[must_use]
    pub fn subkey_count(&self) -> usize {
        match self {
            Self::Dflt { owner_subkeys } => usize::from(*owner_subkeys),
            Self::Smpl {
                owner_subkeys,
                members,
            } => {
                usize::from(*owner_subkeys)
                    + members
                        .iter()
                        .map(|m| usize::from(m.subkeys))
                        .sum::<usize>()
            }
        }
    }
}

/// Which copy an inspect reports on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportScope {
    /// The local copy only.
    Local,
    /// The network's copy, for a get.
    SyncGet,
    /// The network's newer subkeys, for a get.
    UpdateGet,
    /// The network's copy, for a set.
    SyncSet,
    /// The network's newer subkeys, for a set.
    UpdateSet,
}

/// An inspect: per subkey, the local and network sequence numbers, and the
/// subkeys written offline and not yet flushed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecordReport {
    pub subkeys: Vec<u32>,
    pub local_seqs: Vec<Option<u32>>,
    pub network_seqs: Vec<Option<u32>>,
    pub offline_subkeys: Vec<u32>,
}

impl RecordReport {
    /// Subkeys the network holds newer than the local copy.
    #[must_use]
    pub fn newer_on_network(&self) -> Vec<u32> {
        self.subkeys
            .iter()
            .zip(self.local_seqs.iter().zip(&self.network_seqs))
            .filter(|(_, (local, network))| network > local)
            .map(|(subkey, _)| *subkey)
            .collect()
    }
}

/// A durable write that settled: it landed, or a newer value won.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settled {
    pub record_key: String,
    pub subkey: u32,
    pub landed: bool,
}

/// A held write handed to the host at lock, or back to the next session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsentWrite {
    pub record_key: String,
    pub subkey: u32,
    pub data: Vec<u8>,
}

/// A watched record changed.
///
/// **A hint obliges a network read of the hinted subkeys** (plan VU3,
/// Veilid 0.5.8 report marks). Veilid marks every subkey it reports to us
/// as reported and unread, a value-carrying report included
/// (`outbound_watch_manager/outbound_watch_state.rs:51-55,268-273`,
/// `watch_value.rs:1960-1964,2014-2019`). A later range-only report (a
/// storage node coalescing several changed subkeys into one push, which
/// drops the value, or a re-hint) reports only the subkeys not marked and
/// counts a copy on the rest (`outbound_watch_state.rs:251-266`), and
/// sends nothing when none is news (`watch_value.rs:2020-2022`). Only a get
/// that reaches the network clears a mark (`get_value.rs:199-200`,
/// `consume_watch_read_subkeys`, `watch_value.rs:2275-2297`); a local read,
/// or using [`value`](Self::value), leaves it. So a handler reads every
/// subkey in [`subkeys`](Self::subkeys), the one whose value came with the
/// change too, with [`RecordPort::read_hinted`]; otherwise a later change
/// of a subkey it skipped can arrive in a coalesced push and never be
/// reported. Change-inspection reports bypass the marks
/// (`watch_value.rs:1520-1566`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueChange {
    pub key: String,
    pub subkeys: Vec<u32>,
    /// Notifications left on the watch (0: it ended).
    pub count: u32,
    /// The first changed subkey's value, when the network sent one.
    pub value: Option<RecordValue>,
}

/// What a value change meant to the pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueChangeReport {
    /// The pool holds the record (a dead watch is the pool's to re-arm).
    pub held: bool,
    /// The watch ended or died.
    pub watch_died: bool,
    pub subkeys: Vec<u32>,
    pub value: Option<RecordValue>,
}

/// One session's records: borrowed by lease, read, written, inspected and
/// watched; durable writes re-pushed until they land.
#[async_trait::async_trait]
pub trait RecordPort: Send + Sync {
    /// Borrow `key`, writable with the `writer` keypair when given. The
    /// first borrower's `intent` opens the record ([`OpenIntent`]: `Hold`
    /// for our own records and long-lived leases, `Check` for one-shot
    /// reads and probes); a borrow of a record already open keeps that
    /// open, and a writer upgrade re-opens it as `Hold`.
    async fn acquire(
        &self,
        key: &str,
        writer: Option<&str>,
        intent: OpenIntent,
    ) -> Result<LeaseId, PortError>;

    /// Create a record (owned by `owner`, a fresh keypair when `None`) and
    /// hold it writable: the lease, the record key and the owner keypair.
    async fn create(
        &self,
        schema: &RecordSchema,
        owner: Option<&str>,
    ) -> Result<(LeaseId, String, zeroize::Zeroizing<String>), PortError>;

    /// The record key a lease borrows, while it is held.
    fn key_of(&self, lease: LeaseId) -> Option<String>;

    /// The owner public key text of the record a lease borrows, as the
    /// record's descriptor names it: the key the record key hashes and the
    /// descriptor is signed with, so comparing it with an expected key pins
    /// the record to that owner (plan E3.2).
    fn owner_of(&self, lease: LeaseId) -> Result<String, PortError>;

    /// Return a borrow.
    async fn release(&self, lease: LeaseId);

    /// Read a subkey; `force_refresh` asks the network.
    async fn get(
        &self,
        lease: LeaseId,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<RecordValue>, PortError>;

    /// Write a subkey, as `writer` when it differs from the lease's.
    async fn set(
        &self,
        lease: LeaseId,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<&str>,
    ) -> Result<SetOutcome, PortError>;

    /// [`set`](Self::set) for state that must land eventually: a miss is
    /// held and re-pushed until it lands. The writer is kept in memory only
    /// and is not part of [`take_held`](Self::take_held).
    async fn set_durable(
        &self,
        lease: LeaseId,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<&str>,
    ) -> Result<SetOutcome, PortError>;

    /// [`set`](Self::set) for a value of our own record every reader must
    /// reach (a member record's card, routes, op-log head, frontier): once
    /// stored, the same bytes are re-sent at the same seq from 60 s on,
    /// doubling to 30 min, for 24 h or until a newer value replaces them
    /// (plan E3.4 OW1). A miss is the caller's to retry.
    async fn set_republished(
        &self,
        lease: LeaseId,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<&str>,
    ) -> Result<SetOutcome, PortError>;

    /// Stop future re-sends of subkeys whose writer binding has ended.
    fn cancel_republish(&self, record: &str, subkeys: &[u32]);

    /// Inspect the leased record.
    async fn inspect(
        &self,
        lease: LeaseId,
        subkeys: Option<&[u32]>,
        scope: ReportScope,
    ) -> Result<RecordReport, PortError>;

    /// The values of `subkeys` an inspect says exist, fetching from the
    /// network only what changed. Never for a [`ValueChange`]'s subkeys:
    /// a local read leaves Veilid's report mark ([`read_hinted`](Self::read_hinted)).
    async fn read_changed(
        &self,
        lease: LeaseId,
        subkeys: &[u32],
    ) -> Result<Vec<(u32, RecordValue)>, PortError>;

    /// Retry a hinted read for an unmet sequence claim, subject to D8's cap.
    /// Returns no value when the claim is capped or still unconfirmed.
    async fn refetch_hinted(
        &self,
        lease: LeaseId,
        subkey: u32,
        claimed: u64,
    ) -> Result<Option<RecordValue>, PortError>;

    /// [`read_changed`](Self::read_changed)'s hinted mode, for the subkeys
    /// a [`ValueChange`] named: every one is read from the network
    /// (`force_refresh`), which clears its report mark, so the next change
    /// of it is reported even when it arrives in a coalesced push. No
    /// inspect runs first: a hinted subkey holds a value, and an inspect
    /// whose replica has not caught up would answer "nothing newer" and
    /// turn the read local. A subkey whose read fails or finds no value is
    /// left out.
    ///
    /// # Errors
    /// Every read failed: the caller keeps the copy it already has.
    async fn read_hinted(
        &self,
        lease: LeaseId,
        subkeys: &[u32],
    ) -> Result<Vec<(u32, RecordValue)>, PortError>;

    /// Read `subkey` of the leased record from the newest value a
    /// transaction's node set holds ([`DhtBackend::get_newest`]): a gap read,
    /// for a reader that knows a newer value exists than a get returned
    /// (plan E3.3, D23). Nothing is stored locally.
    ///
    /// # Errors
    /// The lease is not held, or the read failed within the retry budget.
    ///
    /// [`DhtBackend::get_newest`]: crate::port::DhtBackend::get_newest
    async fn read_newest(
        &self,
        lease: LeaseId,
        subkey: u32,
    ) -> Result<Option<RecordValue>, PortError>;

    /// Watch `subkeys` of the leased record (empty stops this borrower's
    /// watch).
    async fn watch(&self, lease: LeaseId, subkeys: &[u32]) -> Result<(), PortError>;

    /// Watch every subkey of the leased record.
    async fn watch_all(&self, lease: LeaseId) -> Result<(), PortError>;

    /// The subkey cap of the leased record's values.
    fn subkey_cap(&self, lease: LeaseId) -> Result<usize, PortError>;

    /// Mark the leased record as one that cannot change (plan KA1, ADR
    /// 0011: the community anchor after its GenesisCert, a sealed DFLT(1)
    /// page), so [`rehydrate`](Self::rehydrate) never inspects or fetches it.
    ///
    /// # Errors
    /// The lease is not held.
    fn mark_immutable(&self, lease: LeaseId) -> Result<(), PortError>;

    /// Re-open the leased record with its writer as [`OpenIntent::Hold`],
    /// then fetch the subkeys the network holds newer that no armed watch
    /// of this node covers, and none of an immutable record (D4, plan KA1). The re-open is the
    /// only app-side trigger of Veilid's rehydration, and in 0.5.8 it
    /// queues one only when requested, the record was not created by this
    /// node this session, and the last rehydration finished at least
    /// 300 s ago (`storage_manager/open_record.rs:62-81`); the fetch runs
    /// either way.
    async fn rehydrate(&self, lease: LeaseId) -> Result<(), PortError>;

    /// A value change for the pool to account for (watches it re-arms).
    fn on_value_change(&self, change: &ValueChange) -> ValueChangeReport;

    /// Durable writes as they settle.
    fn settled(&self) -> tokio::sync::broadcast::Receiver<Settled>;

    /// The durable writes still held, as `(record key, subkey)`.
    fn held_writes(&self) -> Vec<(String, u32)>;

    /// Take the held writes (lock), to persist for the next session.
    fn take_held(&self) -> Vec<UnsentWrite>;

    /// Hold writes the last session left unsent, re-pushing them as their
    /// records are borrowed writable.
    fn restore_held(&self, writes: Vec<UnsentWrite>);

    /// Begin lock: release every task waiting on the pool and refuse new
    /// calls; calls in flight run on.
    fn drain(&self);

    /// Admit the lock's own writes on the drained pool.
    fn admit_teardown(&self);

    /// End the session: its records close once the calls in flight have
    /// finished. Returns how many records it held.
    fn end(&self) -> usize;

    /// Read one subkey of `key` on a borrow of its own, opened as
    /// [`OpenIntent::Check`].
    async fn read_once(
        &self,
        key: &str,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<RecordValue>, PortError> {
        let lease = self.acquire(key, None, OpenIntent::Check).await?;
        let read = self.get(lease, subkey, force_refresh).await;
        self.release(lease).await;
        read
    }

    /// [`read_newest`](Self::read_newest) of one subkey of `key` on a
    /// borrow of its own, opened as [`OpenIntent::Check`].
    async fn read_newest_once(
        &self,
        key: &str,
        subkey: u32,
    ) -> Result<Option<RecordValue>, PortError> {
        let lease = self.acquire(key, None, OpenIntent::Check).await?;
        let read = self.read_newest(lease, subkey).await;
        self.release(lease).await;
        read
    }

    /// Write one subkey of `key` on a borrow of its own, as `writer`,
    /// opened as [`OpenIntent::Check`].
    async fn write_once(
        &self,
        key: &str,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<&str>,
    ) -> Result<SetOutcome, PortError> {
        let lease = self.acquire(key, None, OpenIntent::Check).await?;
        let written = self.set(lease, subkey, data, writer).await;
        self.release(lease).await;
        written
    }

    /// [`write_once`](Self::write_once) through
    /// [`set_republished`](Self::set_republished): a write to our own
    /// record that every reader must reach.
    async fn write_own_once(
        &self,
        key: &str,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<&str>,
    ) -> Result<SetOutcome, PortError> {
        let lease = self.acquire(key, None, OpenIntent::Check).await?;
        let written = self.set_republished(lease, subkey, data, writer).await;
        self.release(lease).await;
        written
    }

    /// Inspect `key` on a borrow of its own, opened as
    /// [`OpenIntent::Check`].
    async fn inspect_once(
        &self,
        key: &str,
        subkeys: Option<&[u32]>,
        scope: ReportScope,
    ) -> Result<RecordReport, PortError> {
        let lease = self.acquire(key, None, OpenIntent::Check).await?;
        let report = self.inspect(lease, subkeys, scope).await;
        self.release(lease).await;
        report
    }

    /// The leased record's subkeys the network holds newer than our copy.
    async fn inspect_present(&self, lease: LeaseId) -> Result<Vec<u32>, PortError> {
        Ok(self
            .inspect(lease, None, ReportScope::UpdateGet)
            .await?
            .newer_on_network())
    }

    /// The leased record's subkeys written offline and not yet flushed.
    async fn pending_writes(&self, lease: LeaseId) -> Result<Vec<u32>, PortError> {
        Ok(self
            .inspect(lease, None, ReportScope::Local)
            .await?
            .offline_subkeys)
    }
}

/// The records of a node: a pool per unlocked session.
pub trait RecordStore: Send + Sync {
    /// Start a session's pool.
    ///
    /// # Errors
    /// The node cannot serve records (not started).
    fn start_session(&self) -> Result<Arc<dyn RecordPort>, PortError>;
}

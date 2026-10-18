//! Records: one session's pool of borrowed DHT records, and the store that
//! starts a pool per unlock.

use std::sync::Arc;

use crate::lease::LeaseId;

use super::PortError;

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
    /// Stored at consensus.
    Landed,
    /// The fanout finished below consensus; nothing was stored or queued.
    BelowConsensus,
    /// The node is offline; nothing was stored or queued.
    Offline,
    /// The network already holds a newer value, returned here.
    Superseded(RecordValue),
    /// The value equals the local copy; it was re-pushed and no newer value
    /// came back. It proves nothing about the network copy.
    Unchanged,
}

impl SetOutcome {
    /// The write did not reach consensus, or lost to a newer value.
    #[must_use]
    pub fn missed(&self) -> bool {
        matches!(
            self,
            Self::BelowConsensus | Self::Offline | Self::Superseded(_)
        )
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
    /// Borrow `key`, writable with the `writer` keypair when given.
    async fn acquire(&self, key: &str, writer: Option<&str>) -> Result<LeaseId, PortError>;

    /// Create a record (owned by `owner`, a fresh keypair when `None`) and
    /// hold it writable: the lease, the record key and the owner keypair.
    async fn create(
        &self,
        schema: &RecordSchema,
        owner: Option<&str>,
    ) -> Result<(LeaseId, String, String), PortError>;

    /// The record key a lease borrows, while it is held.
    fn key_of(&self, lease: LeaseId) -> Option<String>;

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

    /// Inspect the leased record.
    async fn inspect(
        &self,
        lease: LeaseId,
        subkeys: Option<&[u32]>,
        scope: ReportScope,
    ) -> Result<RecordReport, PortError>;

    /// The values of `subkeys` an inspect says exist, fetching from the
    /// network only what changed.
    async fn read_changed(
        &self,
        lease: LeaseId,
        subkeys: &[u32],
    ) -> Result<Vec<(u32, RecordValue)>, PortError>;

    /// Watch `subkeys` of the leased record (empty stops this borrower's
    /// watch).
    async fn watch(&self, lease: LeaseId, subkeys: &[u32]) -> Result<(), PortError>;

    /// Watch every subkey of the leased record.
    async fn watch_all(&self, lease: LeaseId) -> Result<(), PortError>;

    /// The subkey cap of the leased record's values.
    fn subkey_cap(&self, lease: LeaseId) -> Result<usize, PortError>;

    /// Re-open the leased record with its writer (the network's only
    /// rehydration trigger), then fetch the subkeys the network holds newer.
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

    /// Read one subkey of `key` on a borrow of its own.
    async fn read_once(
        &self,
        key: &str,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<RecordValue>, PortError> {
        let lease = self.acquire(key, None).await?;
        let read = self.get(lease, subkey, force_refresh).await;
        self.release(lease).await;
        read
    }

    /// Write one subkey of `key` on a borrow of its own, as `writer`.
    async fn write_once(
        &self,
        key: &str,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<&str>,
    ) -> Result<SetOutcome, PortError> {
        let lease = self.acquire(key, None).await?;
        let written = self.set(lease, subkey, data, writer).await;
        self.release(lease).await;
        written
    }

    /// Inspect `key` on a borrow of its own.
    async fn inspect_once(
        &self,
        key: &str,
        subkeys: Option<&[u32]>,
        scope: ReportScope,
    ) -> Result<RecordReport, PortError> {
        let lease = self.acquire(key, None).await?;
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

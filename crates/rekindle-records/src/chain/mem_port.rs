//! An in-memory [`RecordPort`] for the chain's tests: records are maps of subkeys,
//! writes need the record's owner keypair.

use parking_lot::Mutex;
use std::collections::HashMap;

use crate::lease::LeaseId;
use crate::port::{
    PortError, RecordPort, RecordReport, RecordSchema, RecordValue, ReportScope, SetOutcome,
    Settled, UnsentWrite, ValueChange, ValueChangeReport,
};
use crate::schema;

#[derive(Default)]
struct Mem {
    next: u64,
    /// Record key → (owner keypair, subkeys).
    records: HashMap<String, (String, Vec<Option<Vec<u8>>>)>,
    /// Lease → (record key, writer keypair).
    leases: HashMap<u64, (String, Option<String>)>,
    /// Values the network holds newer than the local copy (another writer's).
    remote: HashMap<(String, u32), Vec<u8>>,
    /// A read that goes to the network never answers (an unwritten subkey's
    /// fanout running to its timeout, on an internet that stays unready).
    stall_network: bool,
    /// Reads that went to the network.
    network_reads: usize,
}

#[derive(Default)]
pub(super) struct MemPort(Mutex<Mem>);

fn unused() -> PortError {
    PortError::Node("not used by the chain".into())
}

impl MemPort {
    pub(super) fn network_reads(&self) -> usize {
        self.0.lock().network_reads
    }

    pub(super) fn records(&self) -> usize {
        self.0.lock().records.len()
    }

    pub(super) fn overwrite(&self, key: &str, subkey: u32, data: Vec<u8>) {
        let mut mem = self.0.lock();
        mem.records.get_mut(key).unwrap().1[subkey as usize] = Some(data);
    }

    /// The network holds `data` at `key`/`subkey`, newer than our copy.
    pub(super) fn write_remote(&self, key: &str, subkey: u32, data: Vec<u8>) {
        self.0.lock().remote.insert((key.to_owned(), subkey), data);
    }

    pub(super) fn stall_network(&self, stall: bool) {
        self.0.lock().stall_network = stall;
    }

    fn leased(mem: &Mem, lease: LeaseId) -> Result<String, PortError> {
        mem.leases
            .get(&lease.0)
            .map(|l| l.0.clone())
            .ok_or(PortError::LeaseNotHeld(lease.0))
    }
}

#[async_trait::async_trait]
impl RecordPort for MemPort {
    async fn acquire(
        &self,
        key: &str,
        writer: Option<&str>,
        _: crate::port::OpenIntent,
    ) -> Result<LeaseId, PortError> {
        let mut mem = self.0.lock();
        if !mem.records.contains_key(key) {
            return Err(PortError::Dht(format!("no record {key}")));
        }
        mem.next += 1;
        let id = mem.next;
        mem.leases
            .insert(id, (key.to_owned(), writer.map(str::to_owned)));
        Ok(LeaseId(id))
    }

    async fn create(
        &self,
        schema: &RecordSchema,
        owner: Option<&str>,
    ) -> Result<(LeaseId, String, String), PortError> {
        let mut mem = self.0.lock();
        mem.next += 1;
        let id = mem.next;
        let key = format!("VLD0:record-{id}");
        let owner = owner.map_or_else(|| format!("owner-{id}"), str::to_owned);
        mem.records.insert(
            key.clone(),
            (owner.clone(), vec![None; schema.subkey_count()]),
        );
        mem.leases.insert(id, (key.clone(), Some(owner.clone())));
        Ok((LeaseId(id), key, owner))
    }

    fn key_of(&self, lease: LeaseId) -> Option<String> {
        self.0.lock().leases.get(&lease.0).map(|l| l.0.clone())
    }

    fn owner_of(&self, _: LeaseId) -> Result<String, PortError> {
        Err(unused())
    }

    async fn release(&self, lease: LeaseId) {
        self.0.lock().leases.remove(&lease.0);
    }

    async fn get(
        &self,
        lease: LeaseId,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<RecordValue>, PortError> {
        let stalls = {
            let mut mem = self.0.lock();
            let key = Self::leased(&mem, lease)?;
            let local = mem.records[&key].1[subkey as usize].is_some();
            // Without a local copy a get goes to the network
            // (0.5.8 `get_value.rs:161-169`), as a forced one does.
            let network = force_refresh || !local;
            if network {
                mem.network_reads += 1;
                if let Some(data) = mem.remote.remove(&(key.clone(), subkey)) {
                    mem.records.get_mut(&key).unwrap().1[subkey as usize] = Some(data);
                }
            }
            network && mem.stall_network
        };
        if stalls {
            std::future::pending::<()>().await;
        }
        let mem = self.0.lock();
        let key = Self::leased(&mem, lease)?;
        let (owner, subkeys) = &mem.records[&key];
        Ok(subkeys[subkey as usize].clone().map(|data| RecordValue {
            seq: 0,
            data,
            writer: owner.clone(),
        }))
    }

    async fn set(
        &self,
        lease: LeaseId,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<&str>,
    ) -> Result<SetOutcome, PortError> {
        let mut mem = self.0.lock();
        let (key, held) = mem
            .leases
            .get(&lease.0)
            .cloned()
            .ok_or(PortError::LeaseNotHeld(lease.0))?;
        let owner = mem.records[&key].0.clone();
        if writer.map(str::to_owned).or(held).as_deref() != Some(owner.as_str()) {
            return Err(PortError::NotWritable(key));
        }
        // A replica holding a newer value answers with it, and the set
        // stores it as our copy (0.5.8 `set_value.rs:741-760`).
        if let Some(rival) = mem.remote.remove(&(key.clone(), subkey)) {
            mem.records.get_mut(&key).unwrap().1[subkey as usize] = Some(rival.clone());
            return Ok(SetOutcome::Superseded(RecordValue {
                seq: 1,
                data: rival,
                writer: owner,
            }));
        }
        mem.records.get_mut(&key).unwrap().1[subkey as usize] = Some(data);
        Ok(SetOutcome::Landed)
    }

    async fn set_republished(
        &self,
        lease: LeaseId,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<&str>,
    ) -> Result<SetOutcome, PortError> {
        self.set(lease, subkey, data, writer).await
    }

    async fn set_durable(
        &self,
        _: LeaseId,
        _: u32,
        _: Vec<u8>,
        _: Option<&str>,
    ) -> Result<SetOutcome, PortError> {
        Err(unused())
    }

    async fn inspect(
        &self,
        lease: LeaseId,
        subkeys: Option<&[u32]>,
        scope: ReportScope,
    ) -> Result<RecordReport, PortError> {
        if scope != ReportScope::Local {
            return Err(unused());
        }
        let mem = self.0.lock();
        let key = Self::leased(&mem, lease)?;
        let local = &mem.records[&key].1;
        let subkeys: Vec<u32> = subkeys.map_or_else(
            || (0..u32::try_from(local.len()).unwrap()).collect(),
            <[u32]>::to_vec,
        );
        Ok(RecordReport {
            local_seqs: subkeys
                .iter()
                .map(|s| local[*s as usize].as_ref().map(|_| 0))
                .collect(),
            network_seqs: vec![None; subkeys.len()],
            offline_subkeys: Vec::new(),
            subkeys,
        })
    }

    async fn read_changed(
        &self,
        _: LeaseId,
        _: &[u32],
    ) -> Result<Vec<(u32, RecordValue)>, PortError> {
        Err(unused())
    }

    async fn read_hinted(
        &self,
        _: LeaseId,
        _: &[u32],
    ) -> Result<Vec<(u32, RecordValue)>, PortError> {
        Err(unused())
    }

    async fn read_newest(&self, _: LeaseId, _: u32) -> Result<Option<RecordValue>, PortError> {
        Err(unused())
    }

    async fn watch(&self, _: LeaseId, _: &[u32]) -> Result<(), PortError> {
        Err(unused())
    }

    async fn watch_all(&self, _: LeaseId) -> Result<(), PortError> {
        Err(unused())
    }

    fn subkey_cap(&self, _: LeaseId) -> Result<usize, PortError> {
        Err(unused())
    }

    fn mark_immutable(&self, _: LeaseId) -> Result<(), PortError> {
        Err(unused())
    }

    async fn rehydrate(&self, _: LeaseId) -> Result<(), PortError> {
        Err(unused())
    }

    fn on_value_change(&self, change: &ValueChange) -> ValueChangeReport {
        ValueChangeReport {
            held: false,
            watch_died: false,
            subkeys: change.subkeys.clone(),
            value: None,
        }
    }

    fn settled(&self) -> tokio::sync::broadcast::Receiver<Settled> {
        tokio::sync::broadcast::channel(1).1
    }

    fn held_writes(&self) -> Vec<(String, u32)> {
        Vec::new()
    }

    fn take_held(&self) -> Vec<UnsentWrite> {
        Vec::new()
    }

    fn restore_held(&self, _: Vec<UnsentWrite>) {}

    fn drain(&self) {}

    fn admit_teardown(&self) {}

    fn end(&self) -> usize {
        0
    }
}

/// A port with one participant record: its key and owner keypair.
pub(super) async fn participant() -> (MemPort, String, String) {
    let port = MemPort::default();
    let (lease, key, owner) = port.create(&schema::dm_participant(), None).await.unwrap();
    port.release(lease).await;
    (port, key, owner)
}

pub(super) fn entries(range: std::ops::RangeInclusive<u8>) -> Vec<Vec<u8>> {
    range.map(|i| vec![i; 3]).collect()
}

//! The chain over an in-memory [`RecordPort`]: records are maps of subkeys,
//! writes need the record's owner keypair.

use parking_lot::Mutex;
use std::collections::HashMap;

use rekindle_codec::capnp_codec::chain::{encode_chain_page, ChainPage};
use rekindle_types::dht_layout::dm_participant;

use super::{append, head, read_after, retire_through, ChainError, PagePointer};
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
}

#[derive(Default)]
struct MemPort(Mutex<Mem>);

fn unused() -> PortError {
    PortError::Node("not used by the chain".into())
}

impl MemPort {
    fn records(&self) -> usize {
        self.0.lock().records.len()
    }

    fn overwrite(&self, key: &str, subkey: u32, data: Vec<u8>) {
        let mut mem = self.0.lock();
        mem.records.get_mut(key).unwrap().1[subkey as usize] = Some(data);
    }
}

#[async_trait::async_trait]
impl RecordPort for MemPort {
    async fn acquire(&self, key: &str, writer: Option<&str>) -> Result<LeaseId, PortError> {
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
        _force_refresh: bool,
    ) -> Result<Option<RecordValue>, PortError> {
        let mem = self.0.lock();
        let (key, _) = mem
            .leases
            .get(&lease.0)
            .ok_or(PortError::LeaseNotHeld(lease.0))?;
        let (owner, subkeys) = &mem.records[key];
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
        let (owner, subkeys) = mem.records.get_mut(&key).unwrap();
        if writer.map(str::to_owned).or(held).as_deref() != Some(owner.as_str()) {
            return Err(PortError::NotWritable(key));
        }
        subkeys[subkey as usize] = Some(data);
        Ok(SetOutcome::Landed)
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
        _: LeaseId,
        _: Option<&[u32]>,
        _: ReportScope,
    ) -> Result<RecordReport, PortError> {
        Err(unused())
    }

    async fn read_changed(
        &self,
        _: LeaseId,
        _: &[u32],
    ) -> Result<Vec<(u32, RecordValue)>, PortError> {
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
async fn participant() -> (MemPort, String, String) {
    let port = MemPort::default();
    let (lease, key, owner) = port.create(&schema::dm_participant(), None).await.unwrap();
    port.release(lease).await;
    (port, key, owner)
}

fn entries(range: std::ops::RangeInclusive<u8>) -> Vec<Vec<u8>> {
    range.map(|i| vec![i; 3]).collect()
}

fn seqs(read: &super::ChainRead) -> Vec<u64> {
    read.entries.iter().map(|e| e.seq).collect()
}

#[tokio::test]
async fn appends_read_back_in_order() {
    let (port, key, owner) = participant().await;
    let first = append(&port, &key, &owner, &entries(1..=2)).await.unwrap();
    let second = append(&port, &key, &owner, &entries(3..=5)).await.unwrap();
    assert_eq!((first.first_seq, first.last_seq), (1, 2));
    assert_eq!((second.first_seq, second.last_seq), (3, 5));
    assert_eq!(head(&port, &key).await.unwrap(), Some(second));

    let read = read_after(&port, &key, 0).await.unwrap();
    assert_eq!(seqs(&read), vec![1, 2, 3, 4, 5]);
    assert_eq!(read.entries[4].ciphertext, vec![5; 3]);
    assert_eq!((read.last_seq, read.retired_gap), (5, None));
}

#[tokio::test]
async fn reads_only_after_the_cursor() {
    let (port, key, owner) = participant().await;
    for batch in [1..=2, 3..=4, 5..=6] {
        append(&port, &key, &owner, &entries(batch)).await.unwrap();
    }
    assert_eq!(
        seqs(&read_after(&port, &key, 3).await.unwrap()),
        vec![4, 5, 6]
    );
    assert!(read_after(&port, &key, 6).await.unwrap().entries.is_empty());
    let empty = participant().await;
    assert_eq!(
        read_after(&empty.0, &empty.1, 0).await.unwrap(),
        super::ChainRead::default()
    );
}

#[tokio::test]
async fn retire_stops_the_walk_and_reports_the_gap() {
    let (port, key, owner) = participant().await;
    let first = append(&port, &key, &owner, &entries(1..=2)).await.unwrap();
    for batch in [3..=4, 5..=6] {
        append(&port, &key, &owner, &entries(batch)).await.unwrap();
    }
    retire_through(&port, &key, &owner, 4).await.unwrap();
    // The retired pages are never fetched: break the first one.
    port.overwrite(&first.page_key, 0, vec![0xff; 9]);
    let read = read_after(&port, &key, 0).await.unwrap();
    assert_eq!(seqs(&read), vec![5, 6]);
    assert_eq!(read.retired_gap, Some(4));
    let caught_up = read_after(&port, &key, 5).await.unwrap();
    assert_eq!((seqs(&caught_up), caught_up.retired_gap), (vec![6], None));

    // The mark only moves up, and never past the chain's end.
    retire_through(&port, &key, &owner, 2).await.unwrap();
    assert_eq!(super::retired_through(&port, &key).await.unwrap(), 4);
    assert_eq!(
        retire_through(&port, &key, &owner, 7).await,
        Err(ChainError::RetireBeyondHead {
            seq: 7,
            last_seq: 6
        })
    );
}

#[tokio::test]
async fn pages_are_sealed_one_per_append() {
    let (port, key, owner) = participant().await;
    for i in 0..40u8 {
        append(&port, &key, &owner, &[vec![i]]).await.unwrap();
    }
    assert_eq!(port.records(), 41);
    let read = read_after(&port, &key, 0).await.unwrap();
    assert_eq!(seqs(&read), (1..=40).collect::<Vec<_>>());
    // A page's owner keypair is gone: another writer cannot rewrite it.
    let page = head(&port, &key).await.unwrap().unwrap();
    let rewrite = port
        .write_once(&page.page_key, 0, vec![0], Some(&owner))
        .await;
    assert!(matches!(rewrite, Err(PortError::NotWritable(_))));
    // An empty or oversized append writes nothing.
    assert_eq!(
        append(&port, &key, &owner, &[]).await,
        Err(ChainError::EmptyAppend)
    );
    let big = append(&port, &key, &owner, &[vec![7; 40_000]]).await;
    assert!(matches!(big, Err(ChainError::TooLarge { .. })));
    assert_eq!(port.records(), 41);
}

#[tokio::test]
async fn malformed_pages_are_refused() {
    let (port, key, owner) = participant().await;
    append(&port, &key, &owner, &entries(1..=2)).await.unwrap();
    let newest = append(&port, &key, &owner, &entries(3..=4)).await.unwrap();

    let refuse = |page: Vec<u8>| {
        port.overwrite(&newest.page_key, 0, page);
        read_after(&port, &key, 0)
    };
    assert!(matches!(
        refuse(vec![0xff; 9]).await,
        Err(ChainError::Malformed { .. })
    ));
    // One entry short of its pointer.
    let short = ChainPage {
        prev: None,
        first_seq: 3,
        entries: entries(3..=3),
    };
    assert!(matches!(
        refuse(encode_chain_page(&short)).await,
        Err(ChainError::Malformed { .. })
    ));
    // A link that does not end right before the page.
    let gap = ChainPage {
        prev: Some(PagePointer {
            page_key: "VLD0:x".into(),
            first_seq: 1,
            last_seq: 1,
        }),
        first_seq: 3,
        entries: entries(3..=4),
    };
    assert!(matches!(
        refuse(encode_chain_page(&gap)).await,
        Err(ChainError::Malformed { .. })
    ));
    // A HEAD with an empty range.
    port.overwrite(&key, dm_participant::HEAD, vec![0; 4]);
    assert!(matches!(
        read_after(&port, &key, 0).await,
        Err(ChainError::Malformed { .. })
    ));
}

#[test]
fn page_runs_fill_each_page_in_order() {
    let small: Vec<Vec<u8>> = (1..=20u8).map(|i| vec![i; 300]).collect();
    let runs = super::page_runs(&small).unwrap();
    assert_eq!(runs.len(), 1, "twenty small entries share one page");
    assert_eq!(runs[0], &small[..]);

    let large: Vec<Vec<u8>> = (1..=5u8)
        .map(|i| vec![i; super::page::PAGE_CAP / 3])
        .collect();
    let runs = super::page_runs(&large).unwrap();
    assert!(runs.len() > 1, "large entries span pages");
    assert_eq!(runs.concat(), large, "in order, each entry once");
    for run in &runs {
        assert!(!run.is_empty());
    }

    assert!(matches!(
        super::page_runs(&[vec![0xa5; super::page::PAGE_CAP]]),
        Err(ChainError::TooLarge { .. })
    ));
}

/// A member's op log: the chain's head is `M`/`GOV_HEAD`, nothing else in
/// the record moves, and the log is read without a retire mark.
#[tokio::test]
async fn an_op_log_chains_on_the_member_records_gov_head() {
    use rekindle_types::dht_layout::member_record;
    let port = MemPort::default();
    let (lease, key, owner) = port.create(&schema::member_record(), None).await.unwrap();
    port.release(lease).await;
    let gov_head = member_record::GOV_HEAD;
    super::append_at(&port, &key, gov_head, &owner, &entries(1..=2))
        .await
        .unwrap();
    let newest = super::append_at(&port, &key, gov_head, &owner, &entries(3..=3))
        .await
        .unwrap();
    assert_eq!(
        super::head_at(&port, &key, gov_head).await.unwrap(),
        Some(newest)
    );
    assert_eq!(head(&port, &key).await.unwrap(), None);
    let read = super::read_log_after(&port, &key, gov_head, 1)
        .await
        .unwrap();
    assert_eq!(seqs(&read), vec![2, 3]);
    assert_eq!((read.last_seq, read.retired_gap), (3, None));
}

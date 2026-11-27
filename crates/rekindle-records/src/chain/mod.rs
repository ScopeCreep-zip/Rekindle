//! The append-only chain (plan E2.5, primitive N2): one writer's entries,
//! kept on the DHT as a backward-linked list of sealed pages.
//!
//! - The writer's participant record (`dht_layout::dm_participant`) holds
//!   the chain's `HEAD`: a pointer `{page_key, first_seq, last_seq}` to the
//!   newest page and the retire mark, the highest retired sequence number,
//!   in one value, so a reader's diff costs one fanout (A/B finding R5-5:
//!   a second subkey, empty until the first retire, cost a whole fanout
//!   deadline on every diff). A retire rewrites `HEAD` naming the same page.
//! - Each page is a DFLT(1) record created through [`RecordPort::create`],
//!   written once, and sealed: its owner keypair is dropped after the write
//!   and never stored, so nobody can rewrite it. A page names the page
//!   before it; sequence numbers start at 1 and run without gaps.
//! - Entries are opaque bytes (the caller's ratchet ciphertexts); the chain
//!   never reads them.
//!
//! The reader walks back from `HEAD` to a cursor and stops at the retire
//! mark, reporting when the mark overtook the cursor (the history is gone).
//! It is Veilid-free: everything goes through `&dyn RecordPort`.
//!
//! The same chain carries a member's governance op log (plan E3.2): its
//! head is `M`/`GOV_HEAD` of the member record instead of a participant
//! record's `HEAD`, and it is never retired (a signed DAG op cannot be
//! dropped), so it is written with [`append_at`] and read with
//! [`read_log_after`].

mod create;
mod local;
#[cfg(test)]
mod logged_tests;
#[cfg(test)]
mod mem_port;
mod page;
mod retirement;
#[cfg(test)]
mod tests;

pub use create::{create_participant, publish_participant};
use local::own_head_at;
pub use local::{local_newest_entry, local_pages_after};
pub use rekindle_codec::capnp_codec::chain::PagePointer;
use rekindle_codec::capnp_codec::chain::{ChainHead, ChainPage};
use rekindle_types::dht_layout::dm_participant;
pub use retirement::{
    publish_device_retirement, read_log_with_retirement, DeviceRetirementPointer,
};

use crate::port::{PortError, RecordPort, SetOutcome};
use crate::schema;

/// Rounds of an append whose `HEAD` write a newer `HEAD` superseded: each
/// round chains onto the one that won, as the record pool's read-merge-write
/// appends do (`CAS_ROUNDS`).
const HEAD_ROUNDS: usize = 3;

/// A chain operation failed, or a remote value was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChainError {
    #[error(transparent)]
    Port(#[from] PortError),
    /// An outbound chain value could not be encoded.
    #[error(transparent)]
    Codec(#[from] rekindle_codec::CodecError),
    #[error("an append needs at least one entry")]
    EmptyAppend,
    #[error("{what} of {len} bytes exceeds the {cap}-byte subkey limit")]
    TooLarge {
        what: &'static str,
        len: usize,
        cap: usize,
    },
    /// A page `HEAD` or a page link names is not on the network.
    #[error("chain page {page_key} is missing")]
    PageMissing { page_key: String },
    /// A value read from the network does not parse or does not fit the
    /// chain; nothing in it is used.
    #[error("malformed {what}: {reason}")]
    Malformed { what: &'static str, reason: String },
    /// [`append_exact`]: the chain's next entry is not the assigned one.
    #[error("the chain's next entry is {next}, not the assigned {expected}")]
    SeqMoved { expected: u64, next: u64 },
    #[error("cannot retire through {seq}: the chain ends at {last_seq}")]
    RetireBeyondHead { seq: u64, last_seq: u64 },
}

/// One entry and its sequence number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainEntry {
    pub seq: u64,
    pub ciphertext: Vec<u8>,
}

/// What a read after a cursor found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChainRead {
    /// The entries after the cursor and the retire mark, in sequence order.
    pub entries: Vec<ChainEntry>,
    /// The chain's newest sequence number (0: nothing appended).
    pub last_seq: u64,
    /// `Some(through)` when the retire mark passed the cursor: the entries
    /// after the cursor up to `through` are retired and cannot be fetched.
    pub retired_gap: Option<u64>,
}

/// The newest page, from `HEAD` (`None`: nothing appended).
///
/// # Errors
/// The read failed, or `HEAD` is malformed.
pub async fn head(
    port: &dyn RecordPort,
    participant: &str,
) -> Result<Option<PagePointer>, ChainError> {
    head_at(port, participant, dm_participant::HEAD).await
}

/// The newest page of the chain whose head is `record`/`head_subkey`.
///
/// # Errors
/// The read failed, or the head is malformed.
pub async fn head_at(
    port: &dyn RecordPort,
    record: &str,
    head_subkey: u32,
) -> Result<Option<PagePointer>, ChainError> {
    Ok(read_head(port, record, head_subkey)
        .await?
        .map(|h| h.newest))
}

/// The whole head value of `record`/`head_subkey`, from the network.
async fn read_head(
    port: &dyn RecordPort,
    record: &str,
    head_subkey: u32,
) -> Result<Option<ChainHead>, ChainError> {
    port.read_once(record, head_subkey, true)
        .await?
        .map_or(Ok(None), |v| page::decode_head(&v.data))
}

/// The highest retired sequence number (0: none), from `HEAD`.
///
/// # Errors
/// The read failed, or `HEAD` is malformed.
pub async fn retired_through(port: &dyn RecordPort, participant: &str) -> Result<u64, ChainError> {
    Ok(read_head(port, participant, dm_participant::HEAD)
        .await?
        .map_or(0, |h| h.retired_through))
}

/// Append `entries` as one new page, then point `HEAD` at it. `writer` is
/// the participant record's owner keypair.
///
/// # Errors
/// No entries, a page over the subkey limit, a malformed `HEAD`, or a write
/// that did not land. A page whose `HEAD` write missed is never named and
/// the next append writes a new one.
pub async fn append(
    port: &dyn RecordPort,
    participant: &str,
    writer: &str,
    entries: &[Vec<u8>],
) -> Result<PagePointer, ChainError> {
    append_after(
        port,
        participant,
        dm_participant::HEAD,
        writer,
        None,
        HeadRead::Local,
        FirstHead::Read,
        entries,
    )
    .await
}

/// Where an append reads the head it chains onto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeadRead {
    /// A dm or group chain's `HEAD`, from our copy ([`own_head_at`]).
    Local,
    /// A member's op log head, from the network: every device of the member
    /// holds `K_M(p)` (ADR 0022), so another device may have written the
    /// next seq, and a write at a seq a rival holds is a fork (D15), not a
    /// supersede.
    Network,
}

/// The head an append's first round chains onto.
enum FirstHead {
    /// Read it as [`HeadRead`] says.
    Read,
    /// The value the caller just read ([`append_at_unless_logged`]).
    Known(retirement::HeadValue),
}

/// [`append`] to the chain whose head is `record`/`head_subkey`; `writer`
/// is `record`'s owner keypair. The head is read from the network
/// ([`HeadRead::Network`]): this is the op log's append.
///
/// # Errors
/// As [`append`].
pub async fn append_at(
    port: &dyn RecordPort,
    record: &str,
    head_subkey: u32,
    writer: &str,
    entries: &[Vec<u8>],
) -> Result<PagePointer, ChainError> {
    append_after(
        port,
        record,
        head_subkey,
        writer,
        None,
        HeadRead::Network,
        FirstHead::Read,
        entries,
    )
    .await
}

/// What [`append_at_unless_logged`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoggedAppend {
    /// The entry is the chain's newest page now.
    Appended(PagePointer),
    /// An entry after the cursor already answered `logged`: nothing written.
    AlreadyLogged,
}

/// [`append_at`] of `entry` unless an entry after `cursor` answers
/// `logged` (a retried append whose page an earlier attempt named). One
/// network read of the head serves both the check and the append (plan
/// E3.2 S7c.9): a fresh op log's head is unwritten, so each read of it is
/// a fanout to `get_value_timeout_ms`, 10 s (`veilid_config.rs:1007`), and
/// reading it once for the check and again for the append cost a joiner's
/// `MemberJoined` two of them.
///
/// # Errors
/// As [`append_at`] and [`read_log_after`].
pub async fn append_at_unless_logged(
    port: &dyn RecordPort,
    record: &str,
    head_subkey: u32,
    writer: &str,
    cursor: u64,
    entry: Vec<u8>,
    logged: impl Fn(&[u8]) -> bool,
) -> Result<LoggedAppend, ChainError> {
    let head = retirement::read_head_value(port, record, head_subkey).await?;
    let recent = walk_down(port, head.0.as_ref().map(|h| h.newest.clone()), cursor).await?;
    if recent.iter().any(|e| logged(&e.ciphertext)) {
        return Ok(LoggedAppend::AlreadyLogged);
    }
    append_after(
        port,
        record,
        head_subkey,
        writer,
        None,
        HeadRead::Network,
        FirstHead::Known(head),
        &[entry],
    )
    .await
    .map(LoggedAppend::Appended)
}

/// [`append_at`] with `entries` starting at exactly chain seq `first_seq`,
/// the seq the first one's hint carries (live A/B finding 71 refined): a
/// chain whose next entry is any other is not written.
///
/// # Errors
/// As [`append_at`], or [`ChainError::SeqMoved`].
pub async fn append_exact(
    port: &dyn RecordPort,
    record: &str,
    head_subkey: u32,
    writer: &str,
    first_seq: u64,
    entries: &[Vec<u8>],
) -> Result<PagePointer, ChainError> {
    append_after(
        port,
        record,
        head_subkey,
        writer,
        Some(first_seq),
        HeadRead::Local,
        FirstHead::Read,
        entries,
    )
    .await
}

async fn append_after(
    port: &dyn RecordPort,
    record: &str,
    head_subkey: u32,
    writer: &str,
    expected: Option<u64>,
    read: HeadRead,
    mut first: FirstHead,
    entries: &[Vec<u8>],
) -> Result<PagePointer, ChainError> {
    if entries.is_empty() {
        return Err(ChainError::EmptyAppend);
    }
    let mut round = 1;
    loop {
        // The first round may chain onto a head the caller just read; a
        // superseded round reads it again.
        let (prev, retirement) = match (std::mem::replace(&mut first, FirstHead::Read), read) {
            (FirstHead::Known(head), _) => head,
            (FirstHead::Read, HeadRead::Local) => {
                (own_head_at(port, record, head_subkey).await?, None)
            }
            (FirstHead::Read, HeadRead::Network) => {
                retirement::read_head_value(port, record, head_subkey).await?
            }
        };
        // Carry forward the retire mark and any observed governance locator.
        let retired_through = prev.as_ref().map_or(0, |h| h.retired_through);
        let newest = write_page(port, prev.map(|h| h.newest), expected, entries).await?;
        let head = ChainHead {
            newest,
            retired_through,
        };
        // Our own head moved while the page was written (another append of
        // ours won the race to it): writing ours would replace that entry,
        // so the append is refused and retried at the next seq.
        if let (HeadRead::Local, Some(expected)) = (read, expected) {
            let next = own_head_at(port, record, head_subkey).await?;
            let next = next.map_or(0, |h| h.newest.last_seq) + 1;
            if next != expected {
                return Err(ChainError::SeqMoved { expected, next });
            }
        }
        let outcome = port
            .write_own_once(
                record,
                head_subkey,
                page::encode_head_value(Some(&head), retirement.as_ref())?,
                Some(writer),
            )
            .await?;
        // A newer `HEAD` than our copy won: Veilid stored it as our copy
        // (0.5.8 `set_value.rs:741-746`), so the next round chains onto it. The
        // page this round wrote is never named.
        if matches!(outcome, SetOutcome::Superseded(_)) && round < HEAD_ROUNDS {
            round += 1;
            continue;
        }
        outcome.require_stored(head_subkey)?;
        return Ok(head.newest);
    }
}

/// Write `entries` as the sealed page after `prev`, starting at chain seq
/// `expected` when given.
async fn write_page(
    port: &dyn RecordPort,
    prev: Option<PagePointer>,
    expected: Option<u64>,
    entries: &[Vec<u8>],
) -> Result<PagePointer, ChainError> {
    let first_seq = prev.as_ref().map_or(0, |p| p.last_seq) + 1;
    if let Some(expected) = expected.filter(|e| *e != first_seq) {
        return Err(ChainError::SeqMoved {
            expected,
            next: first_seq,
        });
    }
    let span = u64::try_from(entries.len() - 1).ok();
    let last_seq = span
        .and_then(|span| first_seq.checked_add(span))
        .ok_or_else(|| ChainError::Malformed {
            what: "chain head",
            reason: "sequence numbers overflow".into(),
        })?;
    let bytes = page::encode_page(&ChainPage {
        prev,
        first_seq,
        entries: entries.to_vec(),
    })?;
    Ok(PagePointer {
        page_key: write_sealed_page(port, bytes).await?,
        first_seq,
        last_seq,
    })
}

/// `entries` cut, in order, into the fewest runs that each fit one page
/// whatever page it follows: a writer's queued entries go on the chain one
/// [`append`] per run, so a batch costs a `HEAD` read and two sets per page
/// rather than per entry (plan E2.B, F2).
///
/// # Errors
/// An entry that no page holds alone.
pub fn page_runs(entries: &[Vec<u8>]) -> Result<Vec<&[Vec<u8>]>, ChainError> {
    page::runs(entries)
}

/// Create a DFLT(1) page, write its one subkey, and drop its owner keypair.
async fn write_sealed_page(port: &dyn RecordPort, bytes: Vec<u8>) -> Result<String, ChainError> {
    let (lease, page_key, owner) = port.create(&schema::single(), None).await?;
    // The page is written once with the keypair the lease holds; the
    // keypair is never stored and goes out of memory here.
    drop(owner);
    let written = port.set(lease, 0, bytes, None).await;
    port.release(lease).await;
    written?.require_stored(0)?;
    Ok(page_key)
}

/// The entries after `cursor`, walking back from `HEAD` and stopping at the
/// retire mark.
///
/// # Errors
/// A read failed, a page is missing, or a value is malformed.
pub async fn read_after(
    port: &dyn RecordPort,
    participant: &str,
    cursor: u64,
) -> Result<ChainRead, ChainError> {
    // One value, one fanout: the mark rides in `HEAD` (finding R5-5), and
    // a mark past the newest page is refused as it is decoded.
    let head = read_head(port, participant, dm_participant::HEAD).await?;
    let retired = head.as_ref().map_or(0, |h| h.retired_through);
    let newest = head.map(|h| h.newest);
    let last_seq = newest.as_ref().map_or(0, |p| p.last_seq);
    let floor = cursor.max(retired);
    let entries = walk_down(port, newest, floor).await?;
    Ok(ChainRead {
        entries,
        last_seq,
        retired_gap: (retired > cursor).then_some(retired),
    })
}

/// The entries after `cursor` of a chain that is never retired, whose head
/// is `record`/`head_subkey` (a member's op log).
///
/// # Errors
/// A read failed, a page is missing, or a value is malformed.
pub async fn read_log_after(
    port: &dyn RecordPort,
    record: &str,
    head_subkey: u32,
    cursor: u64,
) -> Result<ChainRead, ChainError> {
    let newest = head_at(port, record, head_subkey).await?;
    read_unretired(port, newest, cursor).await
}

async fn read_unretired(
    port: &dyn RecordPort,
    newest: Option<PagePointer>,
    cursor: u64,
) -> Result<ChainRead, ChainError> {
    let last_seq = newest.as_ref().map_or(0, |p| p.last_seq);
    Ok(ChainRead {
        entries: walk_down(port, newest, cursor).await?,
        last_seq,
        retired_gap: None,
    })
}

/// Gap reads ([`RecordPort::read_newest_once`]) [`read_log_reaching`] runs
/// before it leaves a known head unreached.
pub const GAP_READS: usize = 3;

/// [`read_log_after`] for a reader that knows the chain reaches
/// `at_least` (a departure recorded that head, plan E3.3 D23): when the read
/// stops below it, a get having settled on an older head that the closest
/// replicas still agree on (0.5.8 `get_value.rs:49-88`), the head is read
/// from the newest value a transaction's node set holds, up to
/// [`GAP_READS`] times. The read returned can still stop short; the caller
/// keeps the gap open.
///
/// # Errors
/// As [`read_log_after`]; a failed gap read only counts against the
/// bound.
pub async fn read_log_reaching(
    port: &dyn RecordPort,
    record: &str,
    head_subkey: u32,
    cursor: u64,
    at_least: u64,
) -> Result<ChainRead, ChainError> {
    let read = read_log_after(port, record, head_subkey, cursor).await?;
    if read.last_seq >= at_least {
        return Ok(read);
    }
    for _ in 0..GAP_READS {
        let Ok(Some(value)) = port.read_newest_once(record, head_subkey).await else {
            continue;
        };
        let Some(newest) = page::decode_head(&value.data)?.map(|h| h.newest) else {
            continue;
        };
        if newest.last_seq >= at_least {
            return Ok(ChainRead {
                last_seq: newest.last_seq,
                entries: walk_down(port, Some(newest), cursor).await?,
                retired_gap: None,
            });
        }
    }
    Ok(read)
}

/// The entries above `floor`, in sequence order, walking back from
/// `newest`.
async fn walk_down(
    port: &dyn RecordPort,
    newest: Option<PagePointer>,
    floor: u64,
) -> Result<Vec<ChainEntry>, ChainError> {
    let mut pages: Vec<Vec<ChainEntry>> = Vec::new();
    let mut next = newest;
    // Each page's previous page ends right before it (`page::decode_page`),
    // so the walk moves strictly down and ends.
    while let Some(pointer) = next.take() {
        if pointer.last_seq <= floor {
            break;
        }
        let value = port
            .read_once(&pointer.page_key, 0, false)
            .await?
            .ok_or_else(|| ChainError::PageMissing {
                page_key: pointer.page_key.clone(),
            })?;
        let page = page::decode_page(&value.data, &pointer)?;
        pages.push(
            (page.first_seq..)
                .zip(page.entries)
                .filter(|(seq, _)| *seq > floor)
                .map(|(seq, ciphertext)| ChainEntry { seq, ciphertext })
                .collect(),
        );
        next = page.prev;
    }
    Ok(pages.into_iter().rev().flatten().collect())
}

/// Retire every entry at or below `seq`: readers stop fetching them. The
/// mark only moves up; a lower or equal `seq` writes nothing. `HEAD` is
/// rewritten naming the same newest page with the new mark, from the
/// writer's own copy as an append reads it ([`own_head_at`]); a newer
/// `HEAD` that wins is retired onto in the next round.
///
/// # Errors
/// `seq` is past the chain's end, a value is malformed, or the write did not
/// land.
pub async fn retire_through(
    port: &dyn RecordPort,
    participant: &str,
    writer: &str,
    seq: u64,
) -> Result<(), ChainError> {
    let mut round = 1;
    loop {
        let head = own_head_at(port, participant, dm_participant::HEAD).await?;
        let last_seq = head.as_ref().map_or(0, |h| h.newest.last_seq);
        let Some(head) = head.filter(|_| seq <= last_seq) else {
            return Err(ChainError::RetireBeyondHead { seq, last_seq });
        };
        if seq <= head.retired_through {
            return Ok(());
        }
        let marked = ChainHead {
            newest: head.newest,
            retired_through: seq,
        };
        let outcome = port
            .write_own_once(
                participant,
                dm_participant::HEAD,
                page::encode_head(&marked)?,
                Some(writer),
            )
            .await?;
        if matches!(outcome, SetOutcome::Superseded(_)) && round < HEAD_ROUNDS {
            round += 1;
            continue;
        }
        outcome.require_stored(dm_participant::HEAD)?;
        return Ok(());
    }
}

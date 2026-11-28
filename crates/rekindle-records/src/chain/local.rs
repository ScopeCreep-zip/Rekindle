//! A log's newest entry as our local copy holds it, never read from the
//! network (plan E3.3: the head a departure records of a member's channel
//! outbox, `M`/4). A departure is authored at the state we hold, as the
//! slot heads are (`own_head_at` reads our own head the same way), so a
//! round trip never lets ops arriving meanwhile into its causal past.

use super::{page, ChainEntry, ChainError, ChainHead, PagePointer};
use crate::lease::LeaseId;
use crate::port::{OpenIntent, PortError, RecordPort, RecordValue, ReportScope};

/// The newest entry of the log whose head is `record`/`head_subkey`, from
/// our local copies of the head and of its newest page; `None` when we
/// hold no head or nothing was appended, or the page is not held here.
///
/// # Errors
/// A local read failed, or a value is malformed.
pub async fn local_newest_entry(
    port: &dyn RecordPort,
    record: &str,
    head_subkey: u32,
) -> Result<Option<ChainEntry>, ChainError> {
    let Some(head) = own_head_at(port, record, head_subkey).await? else {
        return Ok(None);
    };
    let pointer = head.newest;
    let lease = port
        .acquire(&pointer.page_key, None, OpenIntent::Check)
        .await?;
    let held = local_value(port, lease, 0).await;
    port.release(lease).await;
    let Some(value) = held? else {
        return Ok(None);
    };
    let mut decoded = page::decode_page(&value.data, &pointer)?;
    // A page's entries end at the pointer's `last_seq` (`decode_page`).
    Ok(decoded.entries.pop().map(|ciphertext| ChainEntry {
        seq: pointer.last_seq,
        ciphertext,
    }))
}

/// The pages of the log whose head is `record`/`head_subkey` holding an
/// entry above `floor`, newest first, from our local copies only (plan
/// E3.2 S7 record warming: a writer re-learns its own sealed op pages at
/// unlock, nothing persisted). The walk follows each page's `prev` down
/// from our own `HEAD` and stops at the first page at or below `floor`, or
/// at one not held here (a page another device of ours wrote and we never
/// read); a local read never goes to the network (`own_head_at`).
///
/// # Errors
/// A local read failed, or a value is malformed.
pub async fn local_pages_after(
    port: &dyn RecordPort,
    record: &str,
    head_subkey: u32,
    floor: u64,
) -> Result<Vec<PagePointer>, ChainError> {
    let mut pages = Vec::new();
    let mut next = own_head_at(port, record, head_subkey)
        .await?
        .map(|head| head.newest);
    // Each page's previous page ends right before it (`decode_page`), so
    // the walk moves strictly down and ends.
    while let Some(pointer) = next.take() {
        if pointer.last_seq <= floor {
            break;
        }
        let lease = port
            .acquire(&pointer.page_key, None, OpenIntent::Check)
            .await?;
        let held = local_value(port, lease, 0).await;
        port.release(lease).await;
        let Some(value) = held? else {
            break;
        };
        next = page::decode_page(&value.data, &pointer)?.prev;
        pages.push(pointer);
    }
    Ok(pages)
}

/// The writer's own `HEAD`, from its local copy, never the network: the
/// append's `HEAD` write is the check against a newer one ([`append_after`]).
///
/// A get with no local copy goes to the network whatever its refresh mode
/// (0.5.8 `get_value.rs:161-169`), and an unwritten subkey has no value to
/// reach consensus on, so that fanout runs to `get_value_timeout_ms`, 10 s
/// (`veilid_config.rs:1007`): a chain's first append waited that long, and
/// past it on an unready internet (the pool's retry waits for readiness),
/// before it wrote anything (E2.5i). The write needs no read first: its seq
/// follows our copy (`set_value.rs:346-385`), and a replica holding a newer
/// value, or another at that seq, answers with it (`set_value.rs:825-845`),
/// which the set stores as our copy and reports as
/// [`SetOutcome::Superseded`] (`set_value.rs:741-760`).
pub(super) async fn own_head_at(
    port: &dyn RecordPort,
    record: &str,
    head_subkey: u32,
) -> Result<Option<ChainHead>, ChainError> {
    let lease = port.acquire(record, None, OpenIntent::Check).await?;
    let local = local_value(port, lease, head_subkey).await;
    port.release(lease).await;
    local?.map_or(Ok(None), |v| page::decode_head(&v.data))
}

/// The leased record's local copy of `subkey`, `None` when there is none.
pub(super) async fn local_value(
    port: &dyn RecordPort,
    lease: LeaseId,
    subkey: u32,
) -> Result<Option<RecordValue>, PortError> {
    let report = port
        .inspect(lease, Some(&[subkey]), ReportScope::Local)
        .await?;
    if report.local_seqs.first().copied().flatten().is_none() {
        return Ok(None);
    }
    port.get(lease, subkey, false).await
}

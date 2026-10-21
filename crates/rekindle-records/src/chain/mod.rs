//! The append-only chain (plan E2.5, primitive N2): one writer's entries,
//! kept on the DHT as a backward-linked list of sealed pages.
//!
//! - The writer's participant record (`dht_layout::dm_participant`) holds
//!   the chain's `HEAD`, a pointer `{page_key, first_seq, last_seq}` to the
//!   newest page, and `RETIRE_THROUGH`, the highest retired sequence number.
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

mod page;
#[cfg(test)]
mod tests;

use rekindle_codec::capnp_codec::chain::ChainPage;
pub use rekindle_codec::capnp_codec::chain::PagePointer;
use rekindle_types::dht_layout::dm_participant;

use crate::port::{PortError, RecordPort};
use crate::schema;

/// A chain operation failed, or a remote value was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChainError {
    #[error(transparent)]
    Port(#[from] PortError),
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
    port.read_once(participant, dm_participant::HEAD, true)
        .await?
        .map(|v| page::decode_head(&v.data))
        .transpose()
}

/// The highest retired sequence number (0: none).
///
/// # Errors
/// The read failed, or the mark is malformed.
pub async fn retired_through(port: &dyn RecordPort, participant: &str) -> Result<u64, ChainError> {
    Ok(port
        .read_once(participant, dm_participant::RETIRE_THROUGH, true)
        .await?
        .map(|v| page::decode_retire(&v.data))
        .transpose()?
        .unwrap_or(0))
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
    if entries.is_empty() {
        return Err(ChainError::EmptyAppend);
    }
    let prev = head(port, participant).await?;
    let first_seq = prev.as_ref().map_or(0, |p| p.last_seq) + 1;
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
    let page_key = write_sealed_page(port, bytes).await?;
    let newest = PagePointer {
        page_key,
        first_seq,
        last_seq,
    };
    port.write_once(
        participant,
        dm_participant::HEAD,
        page::encode_head(&newest)?,
        Some(writer),
    )
    .await?
    .require_stored(dm_participant::HEAD)?;
    Ok(newest)
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
    let newest = head(port, participant).await?;
    let retired = retired_through(port, participant).await?;
    let last_seq = newest.as_ref().map_or(0, |p| p.last_seq);
    if retired > last_seq {
        return Err(ChainError::Malformed {
            what: "chain retire mark",
            reason: format!("retires through {retired}, the chain ends at {last_seq}"),
        });
    }
    let floor = cursor.max(retired);
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
    Ok(ChainRead {
        entries: pages.into_iter().rev().flatten().collect(),
        last_seq,
        retired_gap: (retired > cursor).then_some(retired),
    })
}

/// Retire every entry at or below `seq`: readers stop fetching them. The
/// mark only moves up; a lower or equal `seq` writes nothing.
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
    let last_seq = head(port, participant).await?.map_or(0, |p| p.last_seq);
    if seq > last_seq {
        return Err(ChainError::RetireBeyondHead { seq, last_seq });
    }
    if seq <= retired_through(port, participant).await? {
        return Ok(());
    }
    port.write_once(
        participant,
        dm_participant::RETIRE_THROUGH,
        page::encode_retire(seq)?,
        Some(writer),
    )
    .await?
    .require_stored(dm_participant::RETIRE_THROUGH)?;
    Ok(())
}

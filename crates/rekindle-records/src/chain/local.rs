//! A log's newest entry as our local copy holds it, never read from the
//! network (plan E3.3: the head a departure records of a member's channel
//! outbox, `M`/4). A departure is authored at the state we hold, as the
//! slot heads are (`own_head_at` reads our own head the same way), so a
//! round trip never lets ops arriving meanwhile into its causal past.

use super::{local_value, own_head_at, page, ChainEntry, ChainError, PagePointer};
use crate::port::{OpenIntent, RecordPort};

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

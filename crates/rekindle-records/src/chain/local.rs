//! A log's newest entry as our local copy holds it, never read from the
//! network (plan E3.3: the head a departure records of a member's channel
//! outbox, `M`/4). A departure is authored at the state we hold, as the
//! slot heads are (`own_head_at` reads our own head the same way), so a
//! round trip never lets ops arriving meanwhile into its causal past.

use super::{local_value, own_head_at, page, ChainEntry, ChainError};
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

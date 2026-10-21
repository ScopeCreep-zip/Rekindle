//! A chain page and the pointers to it: encoding under the subkey caps, and
//! the checks a page read from the network must pass before any entry in it
//! is used. Nothing remote is trusted: a page that does not fit its pointer
//! or the page before it is [`ChainError::Malformed`].

use rekindle_codec::capnp_codec::chain::{self as wire, ChainPage, PagePointer};
use rekindle_types::dht_layout::dm_participant;

use super::ChainError;
use crate::lease::{max_subkey_bytes, SchemaShape};

/// The largest page: a DFLT(1) record's only subkey.
pub(super) const PAGE_CAP: usize = max_subkey_bytes(SchemaShape { subkey_count: 1 });

/// The largest participant record value (`HEAD`, `RETIRE_THROUGH`).
pub(super) const PARTICIPANT_CAP: usize = max_subkey_bytes(SchemaShape {
    subkey_count: dm_participant::SUBKEY_COUNT as u32,
});

/// `bytes` fit `cap`, or the value is refused as `what`.
pub(super) fn fits(what: &'static str, bytes: Vec<u8>, cap: usize) -> Result<Vec<u8>, ChainError> {
    if bytes.len() > cap {
        return Err(ChainError::TooLarge {
            what,
            len: bytes.len(),
            cap,
        });
    }
    Ok(bytes)
}

fn malformed(what: &'static str, reason: impl Into<String>) -> ChainError {
    ChainError::Malformed {
        what,
        reason: reason.into(),
    }
}

/// The page's bytes, for its one write.
pub(super) fn encode_page(page: &ChainPage) -> Result<Vec<u8>, ChainError> {
    fits("chain page", wire::encode_chain_page(page), PAGE_CAP)
}

/// The `HEAD` value naming `newest`.
pub(super) fn encode_head(newest: &PagePointer) -> Result<Vec<u8>, ChainError> {
    fits(
        "chain head",
        wire::encode_chain_head(newest),
        PARTICIPANT_CAP,
    )
}

/// The `RETIRE_THROUGH` value.
pub(super) fn encode_retire(through_seq: u64) -> Result<Vec<u8>, ChainError> {
    fits(
        "chain retire mark",
        wire::encode_chain_retire(through_seq),
        PARTICIPANT_CAP,
    )
}

/// A pointer names a page and a non-empty range starting at 1 or later.
fn check_pointer(what: &'static str, p: &PagePointer) -> Result<(), ChainError> {
    if p.page_key.is_empty() {
        return Err(malformed(what, "no page key"));
    }
    if p.first_seq == 0 || p.last_seq < p.first_seq {
        return Err(malformed(
            what,
            format!(
                "range {}..={} is empty or starts at 0",
                p.first_seq, p.last_seq
            ),
        ));
    }
    Ok(())
}

/// A `HEAD` value read from the network.
pub(super) fn decode_head(bytes: &[u8]) -> Result<PagePointer, ChainError> {
    if bytes.len() > PARTICIPANT_CAP {
        return Err(malformed("chain head", "larger than the subkey limit"));
    }
    let head =
        wire::decode_chain_head(bytes).map_err(|e| malformed("chain head", e.to_string()))?;
    check_pointer("chain head", &head)?;
    Ok(head)
}

/// A `RETIRE_THROUGH` value read from the network.
pub(super) fn decode_retire(bytes: &[u8]) -> Result<u64, ChainError> {
    if bytes.len() > PARTICIPANT_CAP {
        return Err(malformed(
            "chain retire mark",
            "larger than the subkey limit",
        ));
    }
    wire::decode_chain_retire(bytes).map_err(|e| malformed("chain retire mark", e.to_string()))
}

/// A page read from the network, checked against the pointer that led to
/// it: the same range, one entry per sequence number, and a previous page
/// that ends right before it (none before sequence 1).
pub(super) fn decode_page(bytes: &[u8], pointer: &PagePointer) -> Result<ChainPage, ChainError> {
    if bytes.len() > PAGE_CAP {
        return Err(malformed("chain page", "larger than the subkey limit"));
    }
    let page =
        wire::decode_chain_page(bytes).map_err(|e| malformed("chain page", e.to_string()))?;
    if page.first_seq != pointer.first_seq {
        return Err(malformed(
            "chain page",
            format!(
                "starts at {}, its pointer at {}",
                page.first_seq, pointer.first_seq
            ),
        ));
    }
    let span = pointer.last_seq - pointer.first_seq + 1;
    if u64::try_from(page.entries.len()).ok() != Some(span) {
        return Err(malformed(
            "chain page",
            format!("{} entries, its pointer names {span}", page.entries.len()),
        ));
    }
    match &page.prev {
        None if page.first_seq == 1 => {}
        None => return Err(malformed("chain page", "no previous page before it")),
        Some(_) if page.first_seq == 1 => {
            return Err(malformed("chain page", "a previous page before sequence 1"))
        }
        Some(prev) => {
            check_pointer("chain page link", prev)?;
            if prev.last_seq != page.first_seq - 1 {
                return Err(malformed(
                    "chain page link",
                    format!(
                        "previous page ends at {}, this one starts at {}",
                        prev.last_seq, page.first_seq
                    ),
                ));
            }
        }
    }
    Ok(page)
}

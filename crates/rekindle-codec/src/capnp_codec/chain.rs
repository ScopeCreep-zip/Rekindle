//! The append-only chain's values (plan E2.5, primitive N2;
//! `schemas/chain.capnp`). This is the wire shape only: whether a page fits
//! its pointer and the page before it is `rekindle_records::chain`'s check.

use super::direct::data;
use super::{bounds, capnp_err, pack, text_to_string, unpack_bounded, CodecError};
use crate::chain_capnp;

/// Where a page sits: its record key and the sequence numbers it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagePointer {
    pub page_key: String,
    pub first_seq: u64,
    pub last_seq: u64,
}

/// A chain's head value: the newest page and the retire mark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainHead {
    pub newest: PagePointer,
    /// Every entry at or below it is retired (0: none).
    pub retired_through: u64,
}

/// One sealed page: its entries from `first_seq` up, and the page before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainPage {
    /// `None` on the chain's first page.
    pub prev: Option<PagePointer>,
    pub first_seq: u64,
    /// Opaque ciphertexts, in sequence order.
    pub entries: Vec<Vec<u8>>,
}

fn set_pointer(mut b: chain_capnp::page_pointer::Builder<'_>, p: &PagePointer) {
    b.set_page_key(p.page_key.as_str());
    b.set_first_seq(p.first_seq);
    b.set_last_seq(p.last_seq);
}

fn read_pointer(r: chain_capnp::page_pointer::Reader<'_>) -> Result<PagePointer, CodecError> {
    Ok(PagePointer {
        page_key: text_to_string(r.get_page_key().map_err(|e| capnp_err(&e))?)?,
        first_seq: r.get_first_seq(),
        last_seq: r.get_last_seq(),
    })
}

/// Participant record subkey `HEAD`: the newest page and the retire mark.
#[must_use]
pub fn encode_chain_head(head: &ChainHead) -> Vec<u8> {
    let mut builder = capnp::message::Builder::new_default();
    {
        let mut root = builder.init_root::<chain_capnp::chain_head::Builder<'_>>();
        root.set_retired_through(head.retired_through);
        set_pointer(root.init_newest(), &head.newest);
    }
    pack(&builder)
}

/// The head a chain's writer puts on the network when it creates the
/// record, before its key leaves: no page yet and nothing retired. Veilid
/// creates a record locally and writes nothing to the network (0.5.8
/// `storage_manager/create_record.rs`), so a reader given the key of a record
/// nobody has written finds no record at all; this value is the first write.
#[must_use]
pub fn encode_empty_chain_head() -> Vec<u8> {
    let mut builder = capnp::message::Builder::new_default();
    builder.init_root::<chain_capnp::chain_head::Builder<'_>>();
    pack(&builder)
}

/// `None`: the empty head ([`encode_empty_chain_head`]), a chain with nothing
/// appended.
///
/// # Errors
/// The bytes are not a `ChainHead`, or a head naming no page retires
/// something.
pub fn decode_chain_head(bytes: &[u8]) -> Result<Option<ChainHead>, CodecError> {
    let reader = unpack_bounded(bytes, bounds::CHAIN_HEAD_BOUND)?;
    let r = reader
        .get_root::<chain_capnp::chain_head::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    if !r.has_newest() {
        if r.get_retired_through() != 0 {
            return Err(CodecError::Deserialization(
                "chain head names no page yet retires entries".into(),
            ));
        }
        return Ok(None);
    }
    Ok(Some(ChainHead {
        newest: read_pointer(r.get_newest().map_err(|e| capnp_err(&e))?)?,
        retired_through: r.get_retired_through(),
    }))
}

/// A page's only subkey.
#[must_use]
pub fn encode_chain_page(page: &ChainPage) -> Vec<u8> {
    let mut builder = capnp::message::Builder::new_default();
    {
        let mut root = builder.init_root::<chain_capnp::chain_page::Builder<'_>>();
        if let Some(prev) = &page.prev {
            set_pointer(root.reborrow().init_prev(), prev);
        }
        root.set_first_seq(page.first_seq);
        let len = u32::try_from(page.entries.len()).unwrap_or(u32::MAX);
        let mut list = root.init_entries(len);
        for (i, entry) in (0..len).zip(&page.entries) {
            list.set(i, entry);
        }
    }
    pack(&builder)
}

/// # Errors
/// The bytes are not a `ChainPage`.
pub fn decode_chain_page(bytes: &[u8]) -> Result<ChainPage, CodecError> {
    let reader = unpack_bounded(bytes, bounds::CHAIN_PAGE_BOUND)?;
    let r = reader
        .get_root::<chain_capnp::chain_page::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    let prev = if r.has_prev() {
        Some(read_pointer(r.get_prev().map_err(|e| capnp_err(&e))?)?)
    } else {
        None
    };
    let list = r.get_entries().map_err(|e| capnp_err(&e))?;
    let entries = list.iter().map(data).collect::<Result<Vec<_>, _>>()?;
    Ok(ChainPage {
        prev,
        first_seq: r.get_first_seq(),
        entries,
    })
}
